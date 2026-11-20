package com.rmusic.nativeapp

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import com.rmusic.core.*
import org.json.JSONObject

data class DownloadItem(
    val id: String,
    val title: String,
    val status: String,
    val bytes: Long = 0,
    val total: Long = 0,
    val error: String = "",
    val namespace: String = "",
    val resumable: Boolean = false,
    val request: String = "",
)

data class DownloadTask(
    val track: Track,
    val namespace: String,
    val directory: String = "",
    val approval: String = "",
    val status: String = "等待",
    val bytes: Long = 0,
    val total: Long = 0,
    val error: String = "",
    val cacheKey: String = "",
    val exported: String = "",
    val validator: String = "",
    val request: String = java.util.UUID.randomUUID().toString(),
) {
    val id
        get() = track.id

    fun item() =
        DownloadItem(
            id,
            track.title,
            status,
            bytes,
            total,
            error,
            namespace,
            status in setOf("已暂停", "等待网络", "已中断，点击继续", "失败"),
            request,
        )

    fun json() =
        json(
            "track" to track.cacheMetadata(),
            "namespace" to namespace,
            "directory" to directory,
            "approval" to approval,
            "status" to status,
            "bytes" to bytes,
            "total" to total,
            "error" to error,
            "key" to cacheKey,
            "exported" to exported,
            "validator" to validator,
            "request" to request,
        )

    companion object {
        fun from(v: JSONObject) =
            DownloadTask(
                Track(v.child("track")),
                v.text("namespace"),
                v.text("directory"),
                v.text("approval"),
                v.text("status"),
                v.optLong("bytes"),
                v.optLong("total"),
                v.text("error"),
                v.text("key"),
                v.text("exported"),
                v.text("validator"),
                v.text("request"),
            )
    }
}

/** Synchronous transactions, called only on IO. Pending transfers are never pruned as history. */
class DownloadJournal(
    context: Context,
    name: String = "download-tasks-v2.db",
    private val capacity: Int = 500,
) : SQLiteOpenHelper(context, name, null, 1) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            "CREATE TABLE tasks(owner TEXT NOT NULL,id TEXT NOT NULL,payload TEXT NOT NULL,touched INTEGER NOT NULL,PRIMARY KEY(owner,id))"
        )
    }

    override fun onUpgrade(db: SQLiteDatabase, old: Int, new: Int) {}

    @Synchronized
    fun put(task: DownloadTask) {
        val existing = get(task.namespace, task.id)
        if (existing == null) {
            var count =
                readableDatabase.rawQuery("SELECT COUNT(*) FROM tasks", null).use {
                    it.moveToFirst()
                    it.getInt(0)
                }
            if (count >= capacity) {
                val completed =
                    readableDatabase
                        .rawQuery("SELECT owner,id,payload FROM tasks ORDER BY touched ASC", null)
                        .use { r ->
                            buildList {
                                while (r.moveToNext()) {
                                    val state = org.json.JSONObject(r.getString(2)).text("status")
                                    if (state in setOf("已缓存", "已导出", "已导出并缓存", "已取消"))
                                        add(r.getString(0) to r.getString(1))
                                }
                            }
                        }
                for ((owner, id) in completed) {
                    writableDatabase.delete("tasks", "owner=? AND id=?", arrayOf(owner, id))
                    count--
                    if (count < capacity) break
                }
            }
            require(count < capacity) { "待处理下载已达到${capacity}条，请等待已有任务完成" }
        }
        writableDatabase.insertWithOnConflict(
            "tasks",
            null,
            ContentValues().apply {
                put("owner", task.namespace)
                put("id", task.id)
                put("payload", task.json().toString())
                put("touched", System.currentTimeMillis())
            },
            SQLiteDatabase.CONFLICT_REPLACE,
        )
    }

    @Synchronized
    fun get(owner: String, id: String): DownloadTask? =
        readableDatabase
            .rawQuery("SELECT payload FROM tasks WHERE owner=? AND id=?", arrayOf(owner, id))
            .use { if (it.moveToFirst()) DownloadTask.from(JSONObject(it.getString(0))) else null }

    @Synchronized
    fun rows(owner: String): List<DownloadTask> =
        readableDatabase
            .rawQuery(
                "SELECT payload FROM tasks WHERE owner=? ORDER BY touched DESC LIMIT 500",
                arrayOf(owner),
            )
            .use { r ->
                buildList {
                    while (r.moveToNext()) add(DownloadTask.from(JSONObject(r.getString(0))))
                }
            }

    @Synchronized
    fun recover() {
        readableDatabase.rawQuery("SELECT payload FROM tasks", null).use { r ->
            while (r.moveToNext()) {
                val t = DownloadTask.from(JSONObject(r.getString(0)))
                if (t.status in setOf("等待", "下载中", "等待网络"))
                    put(t.copy(status = "已中断，点击继续", error = "任务与已验证片段已保留"))
            }
        }
    }
}
