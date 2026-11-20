package com.rmusic.nativeapp

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.*
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.media3.common.util.UnstableApi
import com.rmusic.nativeapp.ui.RMusicRoot

@UnstableApi
class MainActivity : ComponentActivity() {
    override fun onStart() {
        super.onStart()
        (application as RMusicApplication).graph.foreground()
    }

    override fun onStop() {
        (application as RMusicApplication).graph.runtime.checkpoint()
        (application as RMusicApplication).graph.runtime.foreground = false
        super.onStop()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(0xff080d10.toInt()),
        )
        if (android.os.Build.VERSION.SDK_INT >= 29) window.isNavigationBarContrastEnforced = false
        val graph = (application as RMusicApplication).graph
        setContent {
            val api by graph.api.collectAsStateWithLifecycle()
            val permission =
                rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
            LaunchedEffect(api?.session?.cacheNamespace) {
                if (
                    api != null &&
                        graph.preferences.backgroundControl &&
                        android.os.Build.VERSION.SDK_INT >= 33 &&
                        checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) !=
                            android.content.pm.PackageManager.PERMISSION_GRANTED
                )
                    permission.launch(android.Manifest.permission.POST_NOTIFICATIONS)
            }
            RMusicRoot(graph)
        }
    }
}
