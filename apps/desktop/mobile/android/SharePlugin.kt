package chat.commy.mobile

import android.app.Activity
import android.content.Intent
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@TauriPlugin
class SharePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun shareLink(invoke: Invoke) {
    val args = invoke.getArgs()
    val url = args.optString("url")
    if (!url.startsWith("https://") && !url.startsWith("commy://")) {
      invoke.reject("That isn't a link Commy can share")
      return
    }
    val send = Intent(Intent.ACTION_SEND)
      .setType("text/plain")
      .putExtra(Intent.EXTRA_TEXT, url)
    val title = args.optString("title")
    if (title.isNotEmpty()) send.putExtra(Intent.EXTRA_TITLE, title)
    activity.runOnUiThread {
      try {
        activity.startActivity(Intent.createChooser(send, null))
        val answer = JSObject()
        answer.put("shared", true)
        invoke.resolve(answer)
      } catch (e: Exception) {
        invoke.reject("No app on this phone can share the link")
      }
    }
  }
}
