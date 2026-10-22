package chat.commy.mobile

import android.content.Context
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper

class HeadsetRoute(context: Context) {
  private val audio = context.getSystemService(AudioManager::class.java)
  private val main = Handler(Looper.getMainLooper())
  private var watching = false
  private var routedTo: Int? = null

  private val devices = object : AudioDeviceCallback() {
    override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = update()
    override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = update()
  }

  fun start() {
    if (watching) return
    watching = true
    audio.registerAudioDeviceCallback(devices, main)
    update()
  }

  fun stop() {
    if (!watching) return
    watching = false
    audio.unregisterAudioDeviceCallback(devices)
    release()
  }

  private fun update() {
    if (!watching) return
    val headset = headset()
    if (headset == null) {
      release()
      return
    }
    if (routedTo == headset.id) return
    audio.mode = AudioManager.MODE_IN_COMMUNICATION
    if (route(headset)) {
      routedTo = headset.id
    } else {
      routedTo = null
      audio.mode = AudioManager.MODE_NORMAL
    }
  }

  private fun headset(): AudioDeviceInfo? {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      return audio.availableCommunicationDevices.firstOrNull {
        it.type == AudioDeviceInfo.TYPE_BLE_HEADSET || it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO
      }
    }
    return audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).firstOrNull { it.type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO }
  }

  @Suppress("DEPRECATION")
  private fun route(headset: AudioDeviceInfo): Boolean {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) return audio.setCommunicationDevice(headset)
    audio.startBluetoothSco()
    audio.isBluetoothScoOn = true
    return true
  }

  @Suppress("DEPRECATION")
  private fun release() {
    if (routedTo == null) return
    routedTo = null
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      audio.clearCommunicationDevice()
    } else {
      audio.isBluetoothScoOn = false
      audio.stopBluetoothSco()
    }
    audio.mode = AudioManager.MODE_NORMAL
  }
}
