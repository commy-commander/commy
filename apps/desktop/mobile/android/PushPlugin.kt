package chat.commy.mobile

import android.Manifest
import android.app.Activity
import android.content.Intent
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
    WakePreviews.opened()
    WakePreviews.noteTap(activity, activity.intent)
  }

  override fun onNewIntent(intent: Intent) {
    WakePreviews.noteTap(activity, intent)
  }

  override fun onResume() {
    NotificationManagerCompat.from(activity).cancelAll()
    WakePreviews.opened()
  }

  @Command
  fun takeTarget(invoke: Invoke) {
    val answer = JSObject()
    WakePreviews.takeTap()?.let { answer.put("target", it) }
    invoke.resolve(answer)
  }

  @Command
  fun previews(invoke: Invoke) {
    val args = invoke.getArgs().toString()
    Thread {
      try {
        WakePreviews.save(activity.applicationContext, args)
        invoke.resolve()
      } catch (e: Exception) {
        invoke.reject("could not keep the message previews")
      }
    }.start()
  }

  @Command
  fun show(invoke: Invoke) {
    val args = invoke.getArgs()
    val target = args.optString("target").takeIf { WakePreviews.validTarget(it) }
    WakePreviews.postPlain(activity.applicationContext, args.optString("title"), args.optString("body"), target)
    invoke.resolve()
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
