package chat.commy.mobile

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

class CallService : Service() {
  private var wakeLock: PowerManager.WakeLock? = null
  private var wifiLock: WifiManager.WifiLock? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(), serviceType())
    holdLocks()
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    wakeLock?.takeIf { it.isHeld }?.release()
    wifiLock?.takeIf { it.isHeld }?.release()
    wakeLock = null
    wifiLock = null
    super.onDestroy()
  }

  private fun serviceType(): Int {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return 0
    val micAllowed = ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
    return if (micAllowed && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
    } else {
      ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
    }
  }

  private fun notification(): android.app.Notification {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val channel = NotificationChannel(CHANNEL_ID, "Voice calls", NotificationManager.IMPORTANCE_LOW)
      channel.description = "Shown while you are in a voice channel"
      getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }
    val open = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
    val tap = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    return NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(android.R.drawable.ic_btn_speak_now)
      .setContentTitle("Commy")
      .setContentText("You're in a voice channel")
      .setCategory(NotificationCompat.CATEGORY_CALL)
      .setOngoing(true)
      .setSilent(true)
      .setContentIntent(tap)
      .build()
  }

  private fun holdLocks() {
    if (wakeLock == null) {
      val power = getSystemService(PowerManager::class.java)
      wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "commy:voice").apply {
        setReferenceCounted(false)
        acquire(MAX_CALL_MS)
      }
    }
    if (wifiLock == null) {
      val wifi = applicationContext.getSystemService(WifiManager::class.java)
      val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        WifiManager.WIFI_MODE_FULL_LOW_LATENCY
      } else {
        @Suppress("DEPRECATION")
        WifiManager.WIFI_MODE_FULL_HIGH_PERF
      }
      wifiLock = wifi?.createWifiLock(mode, "commy:voice")?.apply {
        setReferenceCounted(false)
        acquire()
      }
    }
  }

  companion object {
    private const val CHANNEL_ID = "voice"
    private const val NOTIFICATION_ID = 7301
    private const val MAX_CALL_MS = 12L * 60 * 60 * 1000
  }
}
