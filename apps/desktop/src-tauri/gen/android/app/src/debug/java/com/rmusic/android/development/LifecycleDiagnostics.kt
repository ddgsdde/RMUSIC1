package com.rmusic.android.development

/** Debug only. Uses a local zero PCM fixture and the production backend. */
object LifecycleDiagnostics {
    @JvmStatic external fun start(input: String)
    @JvmStatic external fun frames(): Long
    @JvmStatic external fun stop()
}
