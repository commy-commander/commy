package chat.commy.mobile

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.Image
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.SystemClock
import android.util.Base64
import android.view.WindowManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import app.tauri.plugin.JSObject
import java.io.ByteArrayOutputStream

object ScreenFrames {
  private var seq = 0L
  private var width = 0
  private var height = 0
  private var jpeg: ByteArray? = null
  private var ended = false

  @Synchronized
  fun begin() {
    seq = 0
    jpeg = null
    ended = false
  }

  @Synchronized
  fun put(frameWidth: Int, frameHeight: Int, bytes: ByteArray) {
    if (ended) return
    seq += 1
    width = frameWidth
    height = frameHeight
    jpeg = bytes
  }

  @Synchronized
  fun end() {
    ended = true
    jpeg = null
  }

  @Synchronized
  fun after(seen: Long): JSObject {
    val answer = JSObject()
    answer.put("seq", seq)
    answer.put("ended", ended)
    val bytes = jpeg
    if (seq > seen && bytes != null) {
      answer.put("width", width)
      answer.put("height", height)
      answer.put("jpeg", Base64.encodeToString(bytes, Base64.NO_WRAP))
    }
    return answer
  }
}

class ScreenShareService : Service() {
  @Volatile private var projection: MediaProjection? = null
  private var display: VirtualDisplay? = null
  private var reader: ImageReader? = null
  private var worker: HandlerThread? = null
  private val lock = Any()
  private var shownWidth = 0
  private var shownHeight = 0
  private var maxWidth = 1280
  private var frameGapMs = 66L
  private var lastFrameAt = 0L

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(), serviceType())
    val data = intent?.let { consent(it) }
    if (intent == null || data == null) {
      ScreenFrames.end()
      stopSelf()
      return START_NOT_STICKY
    }
    release()
    maxWidth = intent.getIntExtra(EXTRA_MAX_WIDTH, 1280).coerceIn(320, 1920)
    frameGapMs = 1000L / intent.getIntExtra(EXTRA_FPS, 15).coerceIn(1, 30)
    try {
      begin(intent.getIntExtra(EXTRA_CODE, 0), data)
    } catch (e: Exception) {
      ScreenFrames.end()
      stopSelf()
    }
    return START_NOT_STICKY
  }

  private fun consent(intent: Intent): Intent? =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      intent.getParcelableExtra(EXTRA_DATA, Intent::class.java)
    } else {
      @Suppress("DEPRECATION")
      intent.getParcelableExtra(EXTRA_DATA)
    }

  override fun onDestroy() {
    release()
    ScreenFrames.end()
    super.onDestroy()
  }

  private fun begin(code: Int, data: Intent) {
    val manager = getSystemService(MediaProjectionManager::class.java)
    val captured = manager.getMediaProjection(code, data) ?: throw IllegalStateException("no screen to share")
    val thread = HandlerThread("commy-screen").apply { start() }
    val handler = Handler(thread.looper)
    worker = thread
    projection = captured
    captured.registerCallback(object : MediaProjection.Callback() {
      override fun onStop() {
        if (projection !== captured) return
        ScreenFrames.end()
        stopSelf()
      }

      override fun onCapturedContentResize(width: Int, height: Int) {
        resize(captured, width, height, handler)
      }
    }, handler)
    val (screenWidth, screenHeight) = screenSize()
    val (width, height) = fit(screenWidth, screenHeight)
    synchronized(lock) {
      val next = newReader(width, height, handler)
      reader = next
      shownWidth = width
      shownHeight = height
      display = captured.createVirtualDisplay(
        "commy-screen",
        width,
        height,
        resources.displayMetrics.densityDpi,
        DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
        next.surface,
        null,
        handler,
      )
    }
  }

  private fun screenSize(): Pair<Int, Int> {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      val bounds = getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
      return Pair(bounds.width(), bounds.height())
    }
    val metrics = resources.displayMetrics
    return Pair(metrics.widthPixels, metrics.heightPixels)
  }

  private fun resize(captured: MediaProjection, contentWidth: Int, contentHeight: Int, handler: Handler) {
    synchronized(lock) {
      if (projection !== captured) return
      val shown = display ?: return
      val (width, height) = fit(contentWidth, contentHeight)
      if (width == shownWidth && height == shownHeight) return
      shownWidth = width
      shownHeight = height
      val old = reader
      val next = newReader(width, height, handler)
      reader = next
      shown.resize(width, height, resources.displayMetrics.densityDpi)
      shown.surface = next.surface
      old?.close()
    }
  }

  private fun fit(width: Int, height: Int): Pair<Int, Int> {
    val scale = minOf(1f, maxWidth.toFloat() / maxOf(width, 1))
    val even = { size: Float -> maxOf(2, (size.toInt() / 2) * 2) }
    return Pair(even(width * scale), even(height * scale))
  }

  private fun newReader(width: Int, height: Int, handler: Handler): ImageReader {
    val next = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 2)
    next.setOnImageAvailableListener({ capture(it) }, handler)
    return next
  }

  private fun capture(source: ImageReader) {
    try {
      source.acquireLatestImage()?.use { encode(it) }
    } catch (e: IllegalStateException) {
      return
    }
  }

  private fun encode(image: Image) {
    val now = SystemClock.elapsedRealtime()
    if (now - lastFrameAt < frameGapMs) return
    lastFrameAt = now
    val plane = image.planes[0]
    val rowPixels = plane.rowStride / plane.pixelStride
    val padded = Bitmap.createBitmap(rowPixels, image.height, Bitmap.Config.ARGB_8888)
    padded.copyPixelsFromBuffer(plane.buffer)
    val frame = if (rowPixels == image.width) padded else Bitmap.createBitmap(padded, 0, 0, image.width, image.height)
    val out = ByteArrayOutputStream()
    frame.compress(Bitmap.CompressFormat.JPEG, JPEG_QUALITY, out)
    if (frame !== padded) frame.recycle()
    padded.recycle()
    ScreenFrames.put(image.width, image.height, out.toByteArray())
  }

  private fun release() {
    val old = synchronized(lock) {
      val stopping = projection
      projection = null
      display?.release()
      display = null
      reader?.close()
      reader = null
      shownWidth = 0
      shownHeight = 0
      stopping
    }
    old?.stop()
    worker?.quitSafely()
    worker = null
  }

  private fun serviceType(): Int =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0

  private fun notification(): android.app.Notification {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val channel = NotificationChannel(CHANNEL_ID, "Screen sharing", NotificationManager.IMPORTANCE_LOW)
      channel.description = "Shown while you are sharing your screen"
      getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }
    val open = Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
    val tap = PendingIntent.getActivity(this, 1, open, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    return NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(android.R.drawable.presence_video_online)
      .setContentTitle("Commy")
      .setContentText("You're sharing your screen")
      .setOngoing(true)
      .setSilent(true)
      .setContentIntent(tap)
      .build()
  }

  companion object {
    const val EXTRA_CODE = "code"
    const val EXTRA_DATA = "data"
    const val EXTRA_MAX_WIDTH = "maxWidth"
    const val EXTRA_FPS = "fps"
    private const val CHANNEL_ID = "screen"
    private const val NOTIFICATION_ID = 7304
    private const val JPEG_QUALITY = 60
  }
}
