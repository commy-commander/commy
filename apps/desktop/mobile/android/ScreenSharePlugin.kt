package chat.commy.mobile

import android.app.Activity
import android.content.Intent
import android.media.projection.MediaProjectionManager
import androidx.activity.result.ActivityResult
import androidx.core.content.ContextCompat
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@InvokeArg
class ShareArgs {
  var maxWidth: Int = 1280
  var fps: Int = 15
}

@InvokeArg
class FrameArgs {
  var after: Long = 0
}

@TauriPlugin
class ScreenSharePlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun start(invoke: Invoke) {
    val manager = activity.getSystemService(MediaProjectionManager::class.java)
    if (manager == null) {
      invoke.reject("This device can't share its screen")
      return
    }
    startActivityForResult(invoke, manager.createScreenCaptureIntent(), "consented")
  }

  @ActivityCallback
  fun consented(invoke: Invoke, result: ActivityResult) {
    val data = result.data
    if (result.resultCode != Activity.RESULT_OK || data == null) {
      invoke.reject("Screen sharing was not allowed")
      return
    }
    val args = invoke.parseArgs(ShareArgs::class.java)
    ScreenFrames.begin()
    val intent = Intent(activity, ScreenShareService::class.java)
      .putExtra(ScreenShareService.EXTRA_CODE, result.resultCode)
      .putExtra(ScreenShareService.EXTRA_DATA, data)
      .putExtra(ScreenShareService.EXTRA_MAX_WIDTH, args.maxWidth)
      .putExtra(ScreenShareService.EXTRA_FPS, args.fps)
    try {
      ContextCompat.startForegroundService(activity, intent)
      invoke.resolve()
    } catch (e: Exception) {
      ScreenFrames.end()
      invoke.reject(e.message ?: "Android would not start screen sharing")
    }
  }

  @Command
  fun frame(invoke: Invoke) {
    val args = invoke.parseArgs(FrameArgs::class.java)
    invoke.resolve(ScreenFrames.after(args.after))
  }

  @Command
  fun stop(invoke: Invoke) {
    activity.stopService(Intent(activity, ScreenShareService::class.java))
    ScreenFrames.end()
    invoke.resolve()
  }
}
