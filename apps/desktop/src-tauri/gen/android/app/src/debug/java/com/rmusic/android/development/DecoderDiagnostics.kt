package com.rmusic.android.development

/** Test harness in debug APKs only; no UI or release entry point. */
object DecoderDiagnostics {
    @JvmStatic external fun activePlatformSources(): Int
    @JvmStatic external fun cancelPlatform(path: String, body: Boolean): Long
    @JvmStatic external fun httpSeek(input: String): ByteArray
    @JvmStatic external fun cancelHttpSeek(body: Boolean): Long
    @JvmStatic external fun install()
    @JvmStatic external fun ready(): Boolean
    @JvmStatic external fun decodeFloat(path: String): ByteArray
    @JvmStatic external fun decode(inputs: String): ByteArray
    @JvmStatic external fun cancelSlow(): Long
    @JvmStatic external fun activeDecoderCount(): Int
    @JvmStatic external fun probe(input: String): String
}
