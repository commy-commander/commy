package chat.commy.mobile

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.core.content.ContextCompat
import androidx.credentials.CredentialManager
import androidx.credentials.CredentialManagerCallback
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetCredentialResponse
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@TauriPlugin
class SignInPlugin(private val activity: Activity) : Plugin(activity) {
  private var sheet: Invoke? = null
  private var away = false

  @Command
  fun openSheet(invoke: Invoke) {
    val url = invoke.getArgs().optString("url")
    if (!url.startsWith("https://")) {
      invoke.reject("That is not a sign-in page")
      return
    }
    activity.runOnUiThread {
      if (sheet != null) {
        invoke.reject("A sign-in page is already open")
        return@runOnUiThread
      }
      sheet = invoke
      away = false
      try {
        CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(activity, Uri.parse(url))
      } catch (e: Exception) {
        sheet = null
        invoke.reject("No browser on this phone could open the sign-in page")
      }
    }
  }

  override fun onPause() {
    if (sheet != null) away = true
  }

  override fun onNewIntent(intent: Intent) {
    val link = intent.data ?: return
    if (link.scheme != RETURN_SCHEME || link.host != RETURN_HOST) return
    val waiting = sheet ?: return
    sheet = null
    away = false
    val answer = JSObject()
    answer.put("url", link.toString())
    waiting.resolve(answer)
  }

  override fun onResume() {
    if (!away) return
    away = false
    val waiting = sheet ?: return
    activity.window.decorView.postDelayed({
      if (sheet === waiting) {
        sheet = null
        waiting.reject("canceled")
      }
    }, CLOSED_SHEET_GRACE_MS)
  }

  @Command
  fun passkey(invoke: Invoke) {
    val options = invoke.getArgs().optString("request")
    if (options.isEmpty()) {
      invoke.reject("The account server sent broken passkey options")
      return
    }
    activity.runOnUiThread {
      val request = GetCredentialRequest(listOf(GetPublicKeyCredentialOption(options)))
      CredentialManager.create(activity).getCredentialAsync(
        activity,
        request,
        null,
        ContextCompat.getMainExecutor(activity),
        object : CredentialManagerCallback<GetCredentialResponse, GetCredentialException> {
          override fun onResult(result: GetCredentialResponse) {
            val credential = result.credential
            if (credential is PublicKeyCredential) {
              val answer = JSObject()
              answer.put("json", credential.authenticationResponseJson)
              invoke.resolve(answer)
            } else {
              invoke.reject("That sign-in is not a passkey")
            }
          }

          override fun onError(e: GetCredentialException) {
            when (e) {
              is GetCredentialCancellationException -> invoke.reject("canceled")
              is NoCredentialException -> invoke.reject("No passkey for this account is saved on this phone")
              else -> invoke.reject(e.message ?: "The passkey did not work")
            }
          }
        },
      )
    }
  }

  companion object {
    const val RETURN_SCHEME = "commy"
    const val RETURN_HOST = "signed-in"
    const val CLOSED_SHEET_GRACE_MS = 800L
  }
}
