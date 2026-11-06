package chat.commy.mobile

import android.Manifest
import android.app.ActivityManager
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
    val sealed = (message.data["previews"] ?: message.data["preview"]).orEmpty().split(",").filter { it.isNotBlank() }
    val shown = sealed.mapNotNull { WakePreviews.open(this, it) }
    Log.i(TAG, "previews opened=${shown.size} of ${sealed.size}")
    if (shown.isNotEmpty()) {
      WakePreviews.postAll(this, shown)
      return
    }
    WakePreviews.ensureChannel(this)
    val notification = NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(android.R.drawable.stat_notify_chat)
      .setContentTitle("Commy")
      .setContentText("New messages in Commy")
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setAutoCancel(true)
      .setWhen(System.currentTimeMillis())
      .setShowWhen(true)
      .setContentIntent(WakePreviews.tapIntent(this, null))
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
