package chat.commy.mobile

import android.app.Activity
import android.app.KeyguardManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.view.WindowManager
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@TauriPlugin
class AppLockPlugin(private val activity: Activity) : Plugin(activity) {
  private var coverWanted = false
  private var asking = false

  @Command
  fun check(invoke: Invoke) {
    val answer = JSObject()
    answer.put("secure", phoneIsLocked())
    answer.put("biometry", biometryName())
    invoke.resolve(answer)
  }

  @Command
  fun unlock(invoke: Invoke) {
    val reason = invoke.getArgs().optString("reason", DEFAULT_REASON)
    activity.runOnUiThread {
      val host = activity as? FragmentActivity
      when {
        !phoneIsLocked() -> invoke.reject(NO_PHONE_LOCK)
        host == null -> invoke.reject("Commy could not show the unlock prompt")
        host.supportFragmentManager.isStateSaved -> invoke.reject(CANCELED)
        asking -> invoke.reject("The unlock prompt is already open")
        else -> ask(host, reason, invoke)
      }
    }
  }

  @Command
  fun cover(invoke: Invoke) {
    val on = invoke.getArgs().optBoolean("on", false)
    activity.runOnUiThread {
      coverWanted = on
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) activity.setRecentsScreenshotEnabled(!on)
      if (!on) activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
      invoke.resolve()
    }
  }

  override fun onPause() {
    if (coverWanted && !asking && Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
      activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }
  }

  override fun onResume() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
      activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
    }
  }

  private fun ask(host: FragmentActivity, reason: String, invoke: Invoke) {
    asking = true
    val answer = object : BiometricPrompt.AuthenticationCallback() {
      override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
        asking = false
        invoke.resolve()
      }

      override fun onAuthenticationError(code: Int, message: CharSequence) {
        asking = false
        invoke.reject(failure(code, message))
      }
    }
    val prompt = BiometricPrompt.PromptInfo.Builder()
      .setTitle(reason)
      .setAllowedAuthenticators(UNLOCK_WAYS)
      .build()
    BiometricPrompt(host, ContextCompat.getMainExecutor(activity), answer).authenticate(prompt)
  }

  private fun phoneIsLocked(): Boolean {
    val keyguard = activity.getSystemService(Context.KEYGUARD_SERVICE) as? KeyguardManager
    return keyguard?.isDeviceSecure == true
  }

  private fun biometryName(): String {
    val enrolled = BiometricManager.from(activity).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_WEAK) == BiometricManager.BIOMETRIC_SUCCESS
    if (!enrolled) return "none"
    val features = activity.packageManager
    val otherKinds = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
      (features.hasSystemFeature(PackageManager.FEATURE_FACE) || features.hasSystemFeature(PackageManager.FEATURE_IRIS))
    return if (features.hasSystemFeature(PackageManager.FEATURE_FINGERPRINT) && !otherKinds) "fingerprint" else "biometrics"
  }

  private fun failure(code: Int, message: CharSequence): String = when (code) {
    BiometricPrompt.ERROR_USER_CANCELED, BiometricPrompt.ERROR_CANCELED, BiometricPrompt.ERROR_NEGATIVE_BUTTON -> CANCELED
    BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL -> NO_PHONE_LOCK
    BiometricPrompt.ERROR_LOCKOUT, BiometricPrompt.ERROR_LOCKOUT_PERMANENT -> "Too many tries. Unlock your phone with its PIN, then try again."
    else -> message.toString()
  }

  companion object {
    const val DEFAULT_REASON = "Unlock Commy"
    const val CANCELED = "canceled"
    const val NO_PHONE_LOCK = "no-lock"
    const val UNLOCK_WAYS = BiometricManager.Authenticators.BIOMETRIC_WEAK or BiometricManager.Authenticators.DEVICE_CREDENTIAL
  }
}
