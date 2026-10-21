package chat.commy.mobile

import android.app.Activity
import android.content.Intent
import android.os.Process
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@TauriPlugin
class RelaunchPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun relaunch(invoke: Invoke) {
    val intent = Intent(activity, RelaunchActivity::class.java)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      .putExtra(RelaunchActivity.OLD_PID, Process.myPid())
    try {
      activity.startActivity(intent)
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message ?: "Commy could not reopen itself")
    }
  }
}
