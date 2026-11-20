package com.rmusic.nativeapp

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import android.util.LruCache
import com.rmusic.core.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** Metadata only: Core's IDs, editions and manual corrections remain authoritative. */
class MusicRepository(context: Context) {
    private class Db(c: Context) : SQLiteOpenHelper(c, "catalog-cache-v1.db", null, 1) {
        override fun onCreate(db: SQLiteDatabase) {
            db.execSQL(
                "CREATE TABLE pages(id TEXT PRIMARY KEY,body TEXT NOT NULL,touched INTEGER NOT NULL)"
            )
        }

        override fun onUpgrade(db: SQLiteDatabase, old: Int, new: Int) {}
    }

    private val db = Db(context)
    private val memory = LruCache<String, String>(24)
    private val gate = Mutex()
    private var api: CoreApi? = null

    fun open(next: CoreApi) {
        api = next
        memory.evictAll()
    }

    fun clearMemory() {
        memory.evictAll()
        api = null
    }

    private fun key(api: CoreApi, path: String) = "${api.session.cacheNamespace}:$path"

    suspend fun cached(path: String): JSONObject? =
        withContext(Dispatchers.IO) {
            val a = api ?: return@withContext null
            val id = key(a, path)
            val text =
                memory[id]
                    ?: db.readableDatabase
                        .rawQuery("SELECT body FROM pages WHERE id=?", arrayOf(id))
                        .use { if (it.moveToFirst()) it.getString(0) else null }
            text?.let { runCatching { JSONObject(it) }.getOrNull() }
        }

    suspend fun refresh(path: String): JSONObject {
        val a = api ?: error("请先连接Core")
        val value = a.request(path)
        val text = value.toString()
        val id = key(a, path)
        gate.withLock {
            withContext(Dispatchers.IO) {
                if (api !== a) return@withContext
                memory.put(id, text)
                val fields =
                    android.content.ContentValues().apply {
                        put("id", id)
                        put("body", text)
                        put("touched", System.currentTimeMillis())
                    }
                db.writableDatabase.insertWithOnConflict(
                    "pages",
                    null,
                    fields,
                    SQLiteDatabase.CONFLICT_REPLACE,
                )
                // At most 100 bounded pages and 16 MiB, independent of audio/image budgets.
                db.writableDatabase.execSQL(
                    "DELETE FROM pages WHERE id NOT IN(SELECT id FROM pages ORDER BY touched DESC LIMIT 100)"
                )
                while (
                    db.readableDatabase
                        .rawQuery("SELECT COALESCE(SUM(length(body)),0) FROM pages", null)
                        .use {
                            it.moveToFirst()
                            it.getLong(0)
                        } > 16 * 1024 * 1024
                ) db.writableDatabase.execSQL(
                    "DELETE FROM pages WHERE id=(SELECT id FROM pages ORDER BY touched LIMIT 1)"
                )
            }
        }
        return value
    }
}
