package com.rmusic.android.development

import android.content.Intent
import android.graphics.Bitmap
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

@RunWith(AndroidJUnit4::class)
class MobileLayoutTest {
    private fun webview(view: View): WebView? {
        if (view is WebView) return view
        if (view is ViewGroup) for (i in 0 until view.childCount) webview(view.getChildAt(i))?.let { return it }
        return null
    }

    @Test fun captureCurrentWindowAndPageGeometry() {
        val runner = InstrumentationRegistry.getInstrumentation()
        val activity = runner.startActivitySync(Intent(runner.targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(20)
        var page: WebView? = null
        while (page == null && System.nanoTime() < deadline) {
            runner.runOnMainSync { page = webview(activity.window.decorView) }
            if (page == null) Thread.sleep(30)
        }
        assertNotNull(page)
        val result = AtomicReference<String>()
        var geometry = JSONObject()
        while (System.nanoTime() < deadline) {
            val ready = CountDownLatch(1)
            runner.runOnMainSync {
                page!!.evaluateJavascript("""(() => {
                  document.querySelector('.main-content')?.scrollTo({top:0,behavior:'instant'});
                  const root=document.querySelector('.app-shell');
                  const rect=e=>{const r=e.getBoundingClientRect(),s=getComputedStyle(e);return {tag:e.tagName,classes:e.className,x:r.x,y:r.y,width:r.width,height:r.height,paddingTop:s.paddingTop,paddingBottom:s.paddingBottom,background:s.backgroundColor,overflow:s.overflow,zIndex:s.zIndex,topbar:s.getPropertyValue('--topbar')}};
                  return {ready:!!root,width:innerWidth,height:innerHeight,dpr:devicePixelRatio,mode:document.documentElement.dataset.mode,root:root?rect(root):null,route:root?.dataset.route,style:root?.dataset.mobileStyle,elements:[...document.querySelectorAll('.main-content,.studio-page-title,.studio-deck,.studio-mast,.music-mobile-header,.scene-player,.mobile-player-top,.studio-player-switch,.scene-stage,.studio-dock,.music-mobile-dock')].map(rect)};
                })()""") { result.set(it); ready.countDown() }
            }
            assertTrue(ready.await(3, TimeUnit.SECONDS))
            geometry = JSONObject(result.get())
            if (geometry.optBoolean("ready")) break
            Thread.sleep(80)
        }
        runner.runOnMainSync {
            val content = activity.findViewById<View>(android.R.id.content)
            val insets = ViewCompat.getRootWindowInsets(content)?.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
            val location = IntArray(2); page!!.getLocationOnScreen(location)
            geometry.put("native", JSONObject().put("contentTop", content.paddingTop).put("contentBottom", content.paddingBottom)
                .put("systemTop", insets?.top).put("systemBottom", insets?.bottom)
                .put("webviewTop", location[1]).put("webviewHeight", page!!.height))
        }
        val directory = File(runner.targetContext.cacheDir, "ui-layout-qa").apply { mkdirs() }
        File(directory, "mobile-layout.json").writeText(geometry.toString(2))
        val image = runner.uiAutomation.takeScreenshot()
        assertNotNull(image)
        File(directory, "mobile-layout.png").outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
        assertTrue("Page never mounted", geometry.optBoolean("ready"))
        if (geometry.optString("route") != "search") {
            assertEquals("System bars must not also pad the webpage", "0px", geometry.getJSONObject("root").getString("paddingTop"))
        }
        assertEquals("WebView must start below the status bar once", geometry.getJSONObject("native").getInt("systemTop"), geometry.getJSONObject("native").getInt("webviewTop"))

        fun query(script: String): JSONObject {
            val done = CountDownLatch(1)
            val value = AtomicReference<String>()
            runner.runOnMainSync { page!!.evaluateJavascript("(() => {$script})()") { value.set(it); done.countDown() } }
            assertTrue(done.await(3, TimeUnit.SECONDS))
            return JSONObject(value.get())
        }
        fun waitFor(condition: String) {
            val until = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
            while (System.nanoTime() < until) {
                if (query("return {ready:!!($condition)}").optBoolean("ready")) return
                Thread.sleep(50)
            }
            fail("Page did not reach $condition")
        }
        fun screenshot(name: String) {
            runner.waitForIdleSync()
            Thread.sleep(450)
            val picture = runner.uiAutomation.takeScreenshot()
            File(directory, "$name.png").outputStream().use { picture.compress(Bitmap.CompressFormat.PNG, 100, it) }
            picture.recycle()
        }
        val backup = query("return {preferences:localStorage.getItem('rmusic.client-preferences.v1')}")
        val layouts = org.json.JSONArray()
        try {
            for ((index, style) in listOf("compact", "classic", "liser", "studio").withIndex()) {
                query("document.querySelector('[aria-label=我的]').click();return {}")
                waitFor("document.querySelector('.studio-menu-links')")
                query("[...document.querySelectorAll('.studio-menu-links button')].find(b=>b.textContent==='设置').click();return {}")
                waitFor("document.querySelector('.settings-scope')")
                query("[...document.querySelectorAll('.settings-scope button')].find(b=>b.textContent.startsWith('客户端设置')).click();return {}")
                waitFor("document.querySelectorAll('.mobile-interface-option').length===4")
                query("document.querySelectorAll('.mobile-interface-option')[$index].click();return {}")
                waitFor("document.querySelector('.app-shell').dataset.mobileStyle==='$style'")
                val border = query("return {shadow:getComputedStyle(document.querySelector('.mobile-interface-option[aria-pressed=true]')).boxShadow}")
                assertTrue("Selection border must fit within its card", border.getString("shadow").contains("inset"))
                query("document.querySelector('nav[aria-label=主导航] button[aria-label=发现]').click();return {}")
                waitFor("document.querySelector('.app-shell').dataset.route==='discover'&&document.querySelector('.studio-deck')")
                val layout = query("""
                    document.querySelector('.main-content').scrollTo({top:0,behavior:'instant'});
                    const root=document.querySelector('.app-shell'),rail=document.querySelector('.studio-deck-rail'),main=document.querySelector('.main-content');
                    return {style:root.dataset.mobileStyle,top:getComputedStyle(root).paddingTop,mainTop:main.getBoundingClientRect().top,overflow:root.scrollWidth>innerWidth,arrows:!!document.querySelector('.studio-flip'),sorts:document.querySelectorAll('.discovery-section .catalog-sort').length,slides:rail.children.length,scrollbar:getComputedStyle(rail,'::-webkit-scrollbar').display};
                """)
                assertEquals("0px", layout.getString("top"))
                assertTrue(layout.getDouble("mainTop") < 1)
                assertFalse(layout.getBoolean("overflow"))
                assertFalse(layout.getBoolean("arrows"))
                assertEquals(0, layout.getInt("sorts"))
                assertEquals("none", layout.getString("scrollbar"))
                layouts.put(layout)
                screenshot("home-$style")
            }
            val selected = query("return {pick:document.querySelector('.studio-deck-dots [aria-pressed=true]').getAttribute('aria-label')}")
            Thread.sleep(6800)
            val advanced = query("return {pick:document.querySelector('.studio-deck-dots [aria-pressed=true]').getAttribute('aria-label')}")
            assertNotEquals("Featured selection must advance automatically", selected.getString("pick"), advanced.getString("pick"))
            val swipe = query("""
                const rail=document.querySelector('.studio-deck-rail');rail.scrollTo({left:0,behavior:'instant'});
                const r=rail.getBoundingClientRect();return {left:(r.x+r.width*.85)*devicePixelRatio,right:(r.x+r.width*.15)*devicePixelRatio,y:(r.y+r.height*.5)*devicePixelRatio};
            """)
            Thread.sleep(150)
            val swipeStart = SystemClock.uptimeMillis()
            for (step in 0..13) {
                val action = if (step == 0) MotionEvent.ACTION_DOWN else if (step == 13) MotionEvent.ACTION_UP else MotionEvent.ACTION_MOVE
                val fraction = minOf(step, 12) / 12f
                runner.runOnMainSync {
                    val x = swipe.getDouble("left") + (swipe.getDouble("right") - swipe.getDouble("left")) * fraction
                    val event = MotionEvent.obtain(swipeStart, SystemClock.uptimeMillis(), action, x.toFloat(), swipe.getDouble("y").toFloat(), 0)
                    page!!.dispatchTouchEvent(event); event.recycle()
                }
                Thread.sleep(25)
            }
            waitFor("document.querySelector('.studio-deck-rail').scrollLeft>document.querySelector('.studio-deck-rail').clientWidth*.7")
            val target = query("""
                const node=document.querySelector('.recommendation-album .recommendation-cover');
                node.scrollIntoView({block:'center',behavior:'instant'});
                const r=node.getBoundingClientRect();return {x:(r.x+r.width/2)*devicePixelRatio,y:(r.y+r.height/2)*devicePixelRatio};
            """)
            Thread.sleep(300)
            val down = SystemClock.uptimeMillis()
            fun touch(action: Int) {
                runner.runOnMainSync {
                    val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, target.getDouble("x").toFloat(), target.getDouble("y").toFloat(), 0)
                    page!!.dispatchTouchEvent(event); event.recycle()
                }
            }
            touch(MotionEvent.ACTION_DOWN); Thread.sleep(600); touch(MotionEvent.ACTION_UP)
            waitFor("document.querySelector('.track-context-menu')")
            assertTrue(query("return {text:document.querySelector('.track-context-menu').textContent}").getString("text").contains("添加到队列末尾"))
            val hit = query("""
                const button=[...document.querySelectorAll('.track-context-menu button')].find(b=>b.textContent==='添加到队列末尾'),r=button.getBoundingClientRect();
                return {clickable:button.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)),width:document.querySelector('.track-context-menu').getBoundingClientRect().width};
            """)
            assertTrue("Backdrop must not block queue actions", hit.getBoolean("clickable"))
            assertEquals(geometry.getInt("width"), hit.getInt("width"))
            screenshot("home-long-press")
            val details = query("""
                const button=[...document.querySelectorAll('.track-context-menu button')].find(b=>b.textContent==='歌曲资料与评论');
                button.scrollIntoView({block:'center',behavior:'instant'});
                const r=button.getBoundingClientRect();return {x:(r.x+r.width/2)*devicePixelRatio,y:(r.y+r.height/2)*devicePixelRatio};
            """)
            Thread.sleep(150)
            runner.runOnMainSync {
                val time = SystemClock.uptimeMillis()
                for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
                    val event = MotionEvent.obtain(time, SystemClock.uptimeMillis(), action, details.getDouble("x").toFloat(), details.getDouble("y").toFloat(), 0)
                    page!!.dispatchTouchEvent(event); event.recycle()
                }
            }
            waitFor("!document.querySelector('.track-context-menu')&&document.querySelector('.modal')")
            File(directory, "layouts.json").writeText(layouts.toString(2))
        } finally {
            if (backup.isNull("preferences")) query("localStorage.removeItem('rmusic.client-preferences.v1');return {}")
            else query("localStorage.setItem('rmusic.client-preferences.v1',${JSONObject.quote(backup.getString("preferences"))});return {}")
            runner.runOnMainSync { page!!.reload() }
            waitFor("document.querySelector('.app-shell')")
        }
    }
}
