package chat.commy.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.util.Base64
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import java.io.File
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.json.JSONObject

data class Shown(val target: String, val sender: String, val text: String, val conversation: String?, val call: Boolean = false)

object WakePreviews {
  const val ACTION_OPEN = "chat.commy.mobile.OPEN_CHAT"
  const val EXTRA_TARGET = "target"
  private const val FILE = "wake-previews.json"
  private const val CHANNEL_ID = "messages"
  private const val CALL_CHANNEL_ID = "calls-ringing"
  private const val RING_FOR_MS = 45_000L
  private const val NONCE_BYTES = 12
  private const val TAG_BITS = 128
  private const val KEEP_LINES = 6
  private const val SAME_FOR_MS = 120_000L
  private val AAD = "commy-wake-preview/v1".toByteArray()
  private val ID = Regex("^[0-9a-fA-F-]{1,64}$")
  private val recent = LinkedHashMap<String, Long>()
  private const val TAG = "CommyWake"
  private const val CALL_ID_MIX = 0x5a5a5a5a

  @Synchronized
  fun save(context: Context, json: String) {
    val file = File(context.filesDir, FILE)
    val next = File(context.filesDir, "$FILE.next")
    next.writeText(json)
    if (!next.renameTo(file)) {
      file.writeText(json)
      next.delete()
    }
  }

  @Synchronized
  private fun load(context: Context): JSONObject? {
    val file = File(context.filesDir, FILE)
    if (!file.exists()) return null
    return try {
      JSONObject(file.readText())
    } catch (e: Exception) {
      null
    }
  }

  fun open(context: Context, sealed: String): Shown? {
    val held = load(context) ?: return null
    val bytes = try {
      Base64.decode(sealed, Base64.DEFAULT)
    } catch (e: IllegalArgumentException) {
      return null
    }
    if (bytes.size <= NONCE_BYTES + TAG_BITS / 8) return null
    val keys = held.optJSONArray("keys") ?: return null
    for (i in 0 until keys.length()) {
      val entry = keys.optJSONObject(i) ?: continue
      val plain = decrypt(entry.optString("k"), bytes) ?: continue
      val preview = try {
        JSONObject(String(plain, Charsets.UTF_8))
      } catch (e: Exception) {
        return null
      }
      return shown(held, entry.optString("s").ifEmpty { null }, preview)
    }
    return null
  }

  private fun decrypt(key: String, bytes: ByteArray): ByteArray? {
    return try {
      val raw = Base64.decode(key, Base64.DEFAULT)
      if (raw.size != 32) return null
      val cipher = Cipher.getInstance("AES/GCM/NoPadding")
      cipher.init(Cipher.DECRYPT_MODE, SecretKeySpec(raw, "AES"), GCMParameterSpec(TAG_BITS, bytes, 0, NONCE_BYTES))
      cipher.updateAAD(AAD)
      cipher.doFinal(bytes, NONCE_BYTES, bytes.size - NONCE_BYTES)
    } catch (e: Exception) {
      null
    }
  }

  private fun shown(held: JSONObject, server: String?, preview: JSONObject): Shown? {
    val author = preview.optString("a")
    val text = preview.optString("t")
    if (!ID.matches(author) || author == held.optString("me")) return null
    if (server == null) {
      val name = held.optJSONObject("users")?.optString(author)?.ifEmpty { null } ?: "Someone"
      return Shown("dm:$author", name, text, null, preview.optBoolean("k"))
    }
    val channel = preview.optString("c")
    if (!ID.matches(server) || !ID.matches(channel)) return null
    val label = held.optJSONObject("servers")?.optJSONObject(server)
    val name = label?.optJSONObject("m")?.optString(author)?.ifEmpty { null } ?: "Someone"
    val serverName = label?.optString("n").orEmpty()
    val channelName = label?.optJSONObject("c")?.optString(channel).orEmpty()
    val conversation = when {
      label?.optBoolean("g") == true -> serverName.ifEmpty { null }
      channelName.isNotEmpty() && serverName.isNotEmpty() -> "#$channelName, $serverName"
      else -> serverName.ifEmpty { null }
    }
    return Shown("channel:$server:$channel", name, text, conversation)
  }

  fun validTarget(target: String?): Boolean {
    if (target == null) return false
    val parts = target.split(":")
    return when {
      parts.size == 3 && parts[0] == "channel" -> ID.matches(parts[1]) && ID.matches(parts[2])
      parts.size == 2 && parts[0] == "dm" -> ID.matches(parts[1])
      else -> false
    }
  }

  @Synchronized
  private fun seenRecently(target: String, text: String): Boolean {
    val now = System.currentTimeMillis()
    recent.entries.removeAll { now - it.value > SAME_FOR_MS }
    val key = "$target\n${sameText(text)}"
    if (recent.containsKey(key)) return true
    recent[key] = now
    return false
  }

  private fun sameText(text: String): String =
    text.filter { it.isLetterOrDigit() }.lowercase().take(40)

  fun ensureChannel(context: Context) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val channel = NotificationChannel(CHANNEL_ID, "New messages", NotificationManager.IMPORTANCE_DEFAULT)
    channel.description = "Shown when something new is waiting while Commy is closed"
    context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
  }

  fun tapIntent(context: Context, target: String?): PendingIntent {
    val open = Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
    if (target != null) {
      open.action = ACTION_OPEN
      open.putExtra(EXTRA_TARGET, target)
    }
    return PendingIntent.getActivity(context, target?.hashCode() ?: 0, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
  }

  fun targetOf(intent: Intent?): String? {
    if (intent == null || intent.action != ACTION_OPEN) return null
    if ((intent.flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY) != 0) return null
    val target = intent.getStringExtra(EXTRA_TARGET)
    intent.removeExtra(EXTRA_TARGET)
    return target?.takeIf { validTarget(it) }
  }

  private fun ensureCallChannel(context: Context) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
    val channel = NotificationChannel(CALL_CHANNEL_ID, "Incoming calls", NotificationManager.IMPORTANCE_HIGH)
    channel.description = "Rings when someone calls you while Commy is closed"
    val sound = AudioAttributes.Builder()
      .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
      .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
      .build()
    channel.setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE), sound)
    channel.enableVibration(true)
    context.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
  }

  private fun postCall(context: Context, shown: Shown) {
    ensureCallChannel(context)
    val tap = tapIntent(context, shown.target)
    val notification = NotificationCompat.Builder(context, CALL_CHANNEL_ID)
      .setSmallIcon(android.R.drawable.stat_sys_phone_call)
      .setContentTitle(shown.sender)
      .setContentText("Calling you")
      .setCategory(NotificationCompat.CATEGORY_CALL)
      .setPriority(NotificationCompat.PRIORITY_HIGH)
      .setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE))
      .setAutoCancel(true)
      .setTimeoutAfter(RING_FOR_MS)
      .setWhen(System.currentTimeMillis())
      .setShowWhen(true)
      .setContentIntent(tap)
      .addAction(android.R.drawable.stat_sys_phone_call, "Answer", tap)
      .build()
    notification.flags = notification.flags or Notification.FLAG_INSISTENT
    try {
      NotificationManagerCompat.from(context).notify(shown.target.hashCode() xor CALL_ID_MIX, notification)
      NotificationManagerCompat.from(context).cancel(WakeService.NOTIFICATION_ID)
    } catch (e: SecurityException) {
      Log.i(TAG, "notifications are not allowed")
    }
  }

  fun post(context: Context, shown: Shown) {
    if (shown.call) {
      postCall(context, shown)
      return
    }
    if (seenRecently(shown.target, shown.text)) return
    ensureChannel(context)
    val id = shown.target.hashCode()
    val manager = NotificationManagerCompat.from(context)
    val earlier = context.getSystemService(NotificationManager::class.java).activeNotifications
      .firstOrNull { it.id == id }
      ?.let { NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(it.notification) }
    val style = NotificationCompat.MessagingStyle(Person.Builder().setName("You").build())
    earlier?.messages?.takeLast(KEEP_LINES - 1)?.forEach { style.addMessage(it) }
    style.addMessage(shown.text.ifEmpty { "New message" }, System.currentTimeMillis(), Person.Builder().setName(shown.sender).build())
    if (shown.conversation != null) {
      style.setConversationTitle(shown.conversation)
      style.setGroupConversation(true)
    }
    val notification = NotificationCompat.Builder(context, CHANNEL_ID)
      .setSmallIcon(android.R.drawable.stat_notify_chat)
      .setStyle(style)
      .setContentTitle(shown.sender)
      .setContentText(shown.text)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setAutoCancel(true)
      .setWhen(System.currentTimeMillis())
      .setShowWhen(true)
      .setContentIntent(tapIntent(context, shown.target))
      .build()
    try {
      manager.notify(id, notification)
      manager.cancel(WakeService.NOTIFICATION_ID)
    } catch (e: SecurityException) {
      Log.i(TAG, "notifications are not allowed")
    }
  }

  fun postPlain(context: Context, title: String, body: String, target: String?) {
    if (target != null && seenRecently(target, body)) return
    ensureChannel(context)
    val id = target?.hashCode() ?: (System.currentTimeMillis() and 0x3fffffff).toInt()
    val notification = NotificationCompat.Builder(context, CHANNEL_ID)
      .setSmallIcon(android.R.drawable.stat_notify_chat)
      .setContentTitle(title)
      .setContentText(body)
      .setStyle(NotificationCompat.BigTextStyle().bigText(body))
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setAutoCancel(true)
      .setWhen(System.currentTimeMillis())
      .setShowWhen(true)
      .setContentIntent(tapIntent(context, target))
      .build()
    try {
      NotificationManagerCompat.from(context).notify(id, notification)
    } catch (e: SecurityException) {
      Log.i(TAG, "notifications are not allowed")
    }
  }
}
