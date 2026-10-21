package chat.commy.mobile

import android.app.Activity
import android.content.Intent
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@TauriPlugin
class CallServicePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun start(invoke: Invoke) {
    try {
      ContextCompat.startForegroundService(activity, Intent(activity, CallService::class.java))
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message ?: "Android would not keep the call running")
    }
  }

  @Command
  fun stop(invoke: Invoke) {
    activity.stopService(Intent(activity, CallService::class.java))
    invoke.resolve()
  }
}
