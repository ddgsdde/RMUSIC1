package com.rmusic.nativeapp

import androidx.test.platform.app.InstrumentationRegistry
import com.rmusic.core.*
import org.junit.Assert.*
import org.junit.Test

class DownloadJournalTest {
    @Test
    fun reopeningKeepsApprovalDestinationAndPausedIntent() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val name = "qa-download-${java.util.UUID.randomUUID()}.db"
        val t = Track(json("id" to "task", "title" to "QA"))
        val first = DownloadJournal(context, name)
        first.put(
            DownloadTask(
                t,
                "owner",
                directory = "content://qa/tree/approved",
                approval = "approval",
                status = "下载中",
                bytes = 500000,
                cacheKey = "same-entity",
            )
        )
        first.put(DownloadTask(Track(json("id" to "paused")), "owner", status = "已暂停"))
        first.put(DownloadTask(t, "other", status = "已缓存"))
        first.close()
        val restored = DownloadJournal(context, name)
        try {
            restored.recover()
            val task = restored.get("owner", "task")!!
            assertEquals("已中断，点击继续", task.status)
            assertEquals("approval", task.approval)
            assertEquals("same-entity", task.cacheKey)
            assertEquals(500000, task.bytes)
            assertEquals("content://qa/tree/approved", task.directory)
            assertEquals("已暂停", restored.get("owner", "paused")!!.status)
            assertEquals("已缓存", restored.get("other", "task")!!.status)
        } finally {
            restored.close()
        }
    }

    @Test
    fun boundedHistoryNeverBlocksFutureDownloadsOrDropsPendingTasks() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val s =
            DownloadJournal(
                context,
                "qa-journal-limit-${java.util.UUID.randomUUID()}.db",
                capacity = 4,
            )
        try {
            for (i in 1..4) s.put(DownloadTask(Track(json("id" to "$i")), "owner", status = "已暂停"))
            val next = DownloadTask(Track(json("id" to "new")), "owner")
            assertTrue(runCatching { s.put(next) }.isFailure)
            val old = s.get("owner", "1")!!
            s.put(old.copy(status = "已缓存"))
            s.put(next)
            assertNotNull(s.get("owner", "new"))
            assertNull(s.get("owner", "1"))
            assertEquals(3, s.rows("owner").count { it.status == "已暂停" })
        } finally {
            s.close()
        }
    }
}
