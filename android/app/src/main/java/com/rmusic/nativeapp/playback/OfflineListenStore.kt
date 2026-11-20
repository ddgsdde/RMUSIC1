package com.rmusic.nativeapp.playback

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import com.rmusic.core.*
import org.json.JSONObject

/** All read/modify/ack steps share this lock, including late network responses. IO threads only. */
class OfflineListenStore(context: Context, name: String = "offline-listens-v1.db") :
    SQLiteOpenHelper(context, name, null, 1) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            "CREATE TABLE listens(id TEXT PRIMARY KEY,owner TEXT NOT NULL,payload TEXT NOT NULL,ack INTEGER NOT NULL DEFAULT 0,error TEXT NOT NULL DEFAULT '')"
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, old: Int, new: Int) {}

    @Synchronized
    fun recover() {
        val c = writableDatabase
        c.beginTransaction()
        try {
            c.rawQuery("SELECT id,payload FROM listens", null).use { r ->
                while (r.moveToNext()) {
                    val v = JSONObject(r.getString(1))
                    if (v.text("reason") == "active") {
                        v.put("reason", "interrupted").put("version", v.optLong("version") + 1)
                        c.execSQL(
                            "UPDATE listens SET payload=? WHERE id=?",
                            arrayOf(v.toString(), r.getString(0)),
                        )
                    }
                }
            }
            c.setTransactionSuccessful()
        } finally {
            c.endTransaction()
        }
    }

    @Synchronized
    fun save(owner: String, snapshot: JSONObject) {
        val c = writableDatabase
        val old =
            c.rawQuery(
                    "SELECT owner,payload,ack,error FROM listens WHERE id=?",
                    arrayOf(snapshot.text("id")),
                )
                .use { r ->
                    if (r.moveToFirst()) {
                        check(r.getString(0) == owner) { "离线记录账号冲突" }
                        Triple(JSONObject(r.getString(1)), r.getLong(2), r.getString(3))
                    } else null
                }
        if (old != null && old.first.optLong("version") >= snapshot.optLong("version")) return
        if (old == null)
            check(
                c.rawQuery("SELECT COUNT(*) FROM listens", null).use {
                    it.moveToFirst()
                    it.getInt(0)
                } < 5000
            ) {
                "离线记录达到5000条，请连接Core同步"
            }
        c.insertWithOnConflict(
            "listens",
            null,
            ContentValues().apply {
                put("id", snapshot.text("id"))
                put("owner", owner)
                put("payload", snapshot.toString())
                put("ack", old?.second ?: 0)
                put("error", old?.third.orEmpty())
            },
            SQLiteDatabase.CONFLICT_REPLACE,
        )
    }

    @Synchronized
    fun pending(owner: String): List<JSONObject> =
        readableDatabase
            .rawQuery("SELECT payload,ack FROM listens WHERE owner=? AND error=''", arrayOf(owner))
            .use { r ->
                buildList {
                    while (r.moveToNext() && size < 32) {
                        val s = JSONObject(r.getString(0))
                        if (s.optLong("version") > r.getLong(1)) add(s)
                    }
                }
            }

    @Synchronized
    fun counts(owner: String): Pair<Int, Int> {
        var pending = 0
        var errors = 0
        readableDatabase
            .rawQuery("SELECT payload,ack,error FROM listens WHERE owner=?", arrayOf(owner))
            .use { r ->
                while (r.moveToNext()) {
                    if (r.getString(2).isNotBlank()) errors++
                    else if (JSONObject(r.getString(0)).optLong("version") > r.getLong(1)) pending++
                }
            }
        return pending to errors
    }

    @Synchronized
    fun acknowledge(owner: String, ack: JSONObject) {
        val c = writableDatabase
        val now =
            c.rawQuery(
                    "SELECT payload FROM listens WHERE owner=? AND id=?",
                    arrayOf(owner, ack.text("id")),
                )
                .use { if (it.moveToFirst()) JSONObject(it.getString(0)) else null } ?: return
        if (ack.has("code")) {
            if (now.optLong("version") == ack.optLong("version"))
                c.execSQL(
                    "UPDATE listens SET error=? WHERE owner=? AND id=?",
                    arrayOf(ack.text("code"), owner, ack.text("id")),
                )
        } else if (
            now.optLong("version") == ack.optLong("version") && now.text("reason") != "active"
        )
            c.delete("listens", "owner=? AND id=?", arrayOf(owner, ack.text("id")))
        else
            c.execSQL(
                "UPDATE listens SET ack=MAX(ack,?) WHERE owner=? AND id=?",
                arrayOf(ack.optLong("version"), owner, ack.text("id")),
            )
    }
}
