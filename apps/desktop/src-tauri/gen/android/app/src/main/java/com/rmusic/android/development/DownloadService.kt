package com.rmusic.android.development

import android.app.*
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import org.json.JSONObject

/** Foreground lifetime for user-requested file transfers; the Rust journal owns recovery. */
class DownloadService : Service() {
 companion object {
  private const val notificationId=4003
  private const val channelId="rmusic-downloads"
  @Volatile private var running=false
  @Volatile private var timedOut=false
  @Synchronized fun update(context:Context,args:JSONObject):String {
   val count=args.getInt("count")
   if(count==0){context.stopService(Intent(context,DownloadService::class.java));return "{\"allowed\":true}"}
   if(timedOut)return "{\"allowed\":false}"
   val intent=Intent(context,DownloadService::class.java).putExtra("count",count)
     .putExtra("received",args.optLong("received")).putExtra("total",args.optLong("total"))
   if(running) context.startService(intent) else if(Build.VERSION.SDK_INT>=26)context.startForegroundService(intent) else context.startService(intent)
   return "{\"allowed\":true}"
  }
 }
 override fun onBind(intent:Intent?):IBinder?=null
 override fun onStartCommand(intent:Intent?,flags:Int,startId:Int):Int {
  val count=intent?.getIntExtra("count",0)?:0
  if(count==0){stopSelf();return START_NOT_STICKY}
  val manager=getSystemService(NotificationManager::class.java)
  manager.createNotificationChannel(NotificationChannel(channelId,"音乐下载",NotificationManager.IMPORTANCE_LOW))
  val launch=PendingIntent.getActivity(this,0,packageManager.getLaunchIntentForPackage(packageName),PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
  val total=intent?.getLongExtra("total",0)?:0
  val received=intent?.getLongExtra("received",0)?:0
  val percent=if(total>0)(received.toDouble()/total*100).toInt().coerceIn(0,100) else 0
  val notification=Notification.Builder(this,channelId).setSmallIcon(applicationInfo.icon)
    .setContentTitle("正在下载音乐").setContentText("$count 项任务 · ${if(total>0) "$percent%" else "正在准备"}")
    .setContentIntent(launch).setOngoing(true).setOnlyAlertOnce(true).setProgress(100,percent,total==0L).build()
  if(Build.VERSION.SDK_INT>=29) startForeground(notificationId,notification,ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
  else startForeground(notificationId,notification)
  running=true
  return START_NOT_STICKY
 }
 override fun onTimeout(startId:Int,fgsType:Int){timedOut=true;stopSelf()}
 override fun onDestroy(){running=false;super.onDestroy()}
}
