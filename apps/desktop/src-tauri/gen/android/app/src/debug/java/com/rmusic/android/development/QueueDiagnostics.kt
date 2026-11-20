package com.rmusic.android.development
object QueueDiagnostics {
    @JvmStatic external fun start(first: String, second: String)
    @JvmStatic external fun state(paused: Boolean): String
    @JvmStatic external fun stall(milliseconds: Long)
    @JvmStatic external fun commit(delayMilliseconds: Long)
    @JvmStatic external fun conflict(): String
    @JvmStatic external fun stop(): String
}
