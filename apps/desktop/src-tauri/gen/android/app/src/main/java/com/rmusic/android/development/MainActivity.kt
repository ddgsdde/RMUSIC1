package com.rmusic.android.development

import android.os.Bundle
import android.animation.ArgbEvaluator
import android.animation.ValueAnimator
import android.view.animation.PathInterpolator
import androidx.core.graphics.ColorUtils
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import android.webkit.JavascriptInterface
import androidx.annotation.Keep
import androidx.activity.enableEdgeToEdge
import androidx.activity.OnBackPressedCallback
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  companion object {
    @JvmStatic external fun restoreNativeWindow()
    @JvmStatic external fun defaultNetworkChanged()
    private var watchingNetwork = false

    @Synchronized private fun watchNetwork(context: android.content.Context) {
      if (watchingNetwork) return
      val manager = context.getSystemService(android.net.ConnectivityManager::class.java)
      manager.registerDefaultNetworkCallback(object : android.net.ConnectivityManager.NetworkCallback() {
        private var active: android.net.Network? = null
        override fun onAvailable(network: android.net.Network) {
          if (active != network) {
            active = network
            defaultNetworkChanged()
          }
        }
        override fun onLost(network: android.net.Network) {
          if (active == network) {
            active = null
            defaultNetworkChanged()
          }
        }
      })
      watchingNetwork = true
    }
  }
  private var page: WebView? = null
  @Volatile private var presentationActive = false
  private var barsAnimator: ValueAnimator? = null
  private var topColour = Color.rgb(18, 18, 18)
  private var bottomColour = Color.rgb(18, 18, 18)
  private val barsBackground = GradientDrawable(GradientDrawable.Orientation.TOP_BOTTOM, intArrayOf(topColour, bottomColour))
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    page = webView
    webView.isVerticalScrollBarEnabled = false
    webView.isHorizontalScrollBarEnabled = false
    webView.addJavascriptInterface(Appearance(), "RMusicAppearance")
  }
  @Keep
  inner class Appearance {
    @JavascriptInterface
    fun isVisible(): Boolean = presentationActive
    @JavascriptInterface
    fun set(colour: String, light: Boolean) {
      setBars(colour, colour, light)
    }
    @JavascriptInterface
    fun setBars(colour: String, navigation: String, light: Boolean) {
      if (!Regex("^#[0-9a-fA-F]{6}$").matches(colour) || !Regex("^#[0-9a-fA-F]{6}$").matches(navigation)) return
      runOnUiThread {
        val colourValue = Color.parseColor(colour)
        val navigationValue = Color.parseColor(navigation)
        barsAnimator?.cancel()
        val fromTop = topColour
        val fromBottom = bottomColour
        val content = findViewById<View>(android.R.id.content)
        content.background = barsBackground
        if (android.os.Build.VERSION.SDK_INT >= 29) {
          window.isStatusBarContrastEnforced = false
          window.isNavigationBarContrastEnforced = false
        }
        fun paint(top: Int, bottom: Int, settled: Boolean) {
          topColour = top; bottomColour = bottom
          barsBackground.colors = intArrayOf(top, bottom)
          window.statusBarColor = top; window.navigationBarColor = bottom
          WindowInsetsControllerCompat(window, window.decorView).apply {
            isAppearanceLightStatusBars = if (settled) light else ColorUtils.calculateLuminance(top) > 0.45
            isAppearanceLightNavigationBars = if (settled) light else ColorUtils.calculateLuminance(bottom) > 0.45
          }
        }
        if (!ValueAnimator.areAnimatorsEnabled() || fromTop == colourValue && fromBottom == navigationValue) {
          paint(colourValue, navigationValue, true)
        } else {
          val evaluator = ArgbEvaluator()
          barsAnimator = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = 300
            interpolator = PathInterpolator(0.215f, 0.61f, 0.355f, 1f)
            addUpdateListener { animator ->
              val fraction = animator.animatedValue as Float
              paint(evaluator.evaluate(fraction, fromTop, colourValue) as Int,
                evaluator.evaluate(fraction, fromBottom, navigationValue) as Int, fraction >= 1f)
            }
            start()
          }
        }
      }
    }
  }
  private external fun activityVisible(visible: Boolean)
  private fun presentationVisibility(visible: Boolean) {
    activityVisible(visible)
    presentationActive = visible
    page?.evaluateJavascript("window.dispatchEvent(new Event('rmusic-activity-visibility'))", null)
  }
  override fun onResume() { super.onResume(); presentationVisibility(true) }
  override fun onPause() { presentationVisibility(false); super.onPause() }
  override fun onDestroy() {
    barsAnimator?.cancel(); barsAnimator = null
    super.onDestroy()
    val previous = page
    page = null
    (previous?.parent as? ViewGroup)?.removeView(previous)
    previous?.removeJavascriptInterface("RMusicAppearance")
    previous?.destroy()
  }
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    watchNetwork(applicationContext)
    restoreNativeWindow()
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        val current = page
        if (current == null) { moveTaskToBack(true); return }
        current.evaluateJavascript("window.dispatchEvent(new CustomEvent('rmusic-system-back',{cancelable:true}))") { result ->
          if (result != "false") moveTaskToBack(true)
        }
      }
    })
    val content = findViewById<View>(android.R.id.content)
    content.setBackgroundColor(Color.rgb(18, 18, 18))
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
      val keyboard = insets.getInsets(WindowInsetsCompat.Type.ime())
      view.setPadding(bars.left, bars.top, bars.right, maxOf(bars.bottom, keyboard.bottom))
      WindowInsetsCompat.CONSUMED
    }
    WindowInsetsControllerCompat(window, window.decorView).apply {
      isAppearanceLightStatusBars = false
      isAppearanceLightNavigationBars = false
    }
    ViewCompat.requestApplyInsets(content)
  }
}
