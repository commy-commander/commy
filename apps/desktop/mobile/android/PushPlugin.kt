package chat.commy.mobile

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging

@TauriPlugin
class PushPlugin(private val activity: Activity) : Plugin(activity) {
  init {
    NotificationManagerCompat.from(activity).cancelAll()
  }

  override fun onResume() {
    NotificationManagerCompat.from(activity).cancelAll()
  }

  @Command
  fun token(invoke: Invoke) {
    NotificationManagerCompat.from(activity).cancel(WakeService.NOTIFICATION_ID)
    if (FirebaseApp.getApps(activity).isEmpty()) {
      invoke.resolve(JSObject())
      return
    }
    askToNotify()
    FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
      val answer = JSObject()
      if (task.isSuccessful && !task.result.isNullOrEmpty()) answer.put("token", task.result)
      invoke.resolve(answer)
    }
  }

  private fun askToNotify() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
    if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
    activity.runOnUiThread {
      ActivityCompat.requestPermissions(activity, arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQUEST_NOTIFY)
    }
  }

  companion object {
    private const val REQUEST_NOTIFY = 7302
  }
}
