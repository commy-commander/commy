package chat.commy.mobile

import android.Manifest
import android.app.ActivityManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

class WakeService : FirebaseMessagingService() {
  override fun onMessageReceived(message: RemoteMessage) {
    if (message.data["wake"] != "1") return
    val foreground = inForeground()
    val allowed = mayNotify()
    Log.i(TAG, "wake-up push received, foreground=$foreground, notifications allowed=$allowed")
    if (foreground || !allowed) return
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val channel = NotificationChannel(CHANNEL_ID, "New messages", NotificationManager.IMPORTANCE_DEFAULT)
      channel.description = "Shown when something new is waiting while Commy is closed"
      getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }
    val open = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
    val tap = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    val notification = NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(android.R.drawable.stat_notify_chat)
      .setContentTitle("Commy")
      .setContentText("New messages in Commy")
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setAutoCancel(true)
      .setOnlyAlertOnce(true)
      .setContentIntent(tap)
      .build()
    NotificationManagerCompat.from(this).notify(NOTIFICATION_ID, notification)
  }

  private fun inForeground(): Boolean {
    val state = ActivityManager.RunningAppProcessInfo()
    ActivityManager.getMyMemoryState(state)
    return state.importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
  }

  private fun mayNotify(): Boolean {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return true
    return ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
  }

  companion object {
    const val NOTIFICATION_ID = 7303
    private const val CHANNEL_ID = "messages"
    private const val TAG = "CommyWake"
  }
}
