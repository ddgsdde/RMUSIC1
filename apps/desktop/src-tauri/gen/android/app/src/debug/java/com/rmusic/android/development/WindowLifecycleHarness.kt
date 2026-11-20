package com.rmusic.android.development

import android.app.Activity
import android.app.Application
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Bundle
import org.json.JSONObject
import java.lang.ref.WeakReference

/** Debug only: finish the actual main Activity while the production queue keeps running. */
class WindowTestApplication : Application() {
    companion object {
        var current = WeakReference<Activity>(null)
        var destroyed = 0
    }
    override fun onCreate() {
        super.onCreate()
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            override fun onActivityCreated(activity: Activity, state: Bundle?) { if (activity is MainActivity) current = WeakReference(activity) }
            override fun onActivityDestroyed(activity: Activity) { if (activity is MainActivity) { destroyed++; if (current.get() === activity) current.clear() } }
            override fun onActivityStarted(activity: Activity) {}
            override fun onActivityResumed(activity: Activity) {}
            override fun onActivityPaused(activity: Activity) {}
            override fun onActivityStopped(activity: Activity) {}
            override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) {}
        })
    }
}
class WindowControlReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val activity = WindowTestApplication.current.get()
        if (intent.action == "com.rmusic.android.development.QA_FINISH_MAIN") activity?.finish()
        resultData = JSONObject().put("active", activity != null && !activity.isDestroyed)
            .put("destroyed", WindowTestApplication.destroyed).toString()
    }
}
