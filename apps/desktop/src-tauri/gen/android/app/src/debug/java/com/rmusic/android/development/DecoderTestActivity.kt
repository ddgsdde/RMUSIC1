package com.rmusic.android.development

import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity

/** Decoder tests avoid starting/destroying the Tauri window runtime. */
class DecoderTestActivity : AppCompatActivity() {
    companion object { init { System.loadLibrary("rmusic_desktop") } }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        AudioPlugin(this)
        DecoderDiagnostics.install()
    }
}
