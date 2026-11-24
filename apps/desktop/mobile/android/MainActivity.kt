package chat.commy.mobile

import android.content.Intent
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import androidx.core.view.WindowCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    matchSystemBarIconsToTheme(resources.configuration)
    WakePreviews.noteTap(this, intent)
  }

  override fun onNewIntent(intent: Intent) {
    WakePreviews.noteTap(this, intent)
    super.onNewIntent(intent)
  }

  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    matchSystemBarIconsToTheme(newConfig)
  }

  @Suppress("OVERRIDE_DEPRECATION")
  override fun onBackPressed() {
    moveTaskToBack(true)
  }

  private fun matchSystemBarIconsToTheme(config: Configuration) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return
    val lightTheme = (config.uiMode and Configuration.UI_MODE_NIGHT_MASK) != Configuration.UI_MODE_NIGHT_YES
    WindowCompat.getInsetsController(window, window.decorView).run {
      isAppearanceLightStatusBars = lightTheme
      isAppearanceLightNavigationBars = lightTheme
    }
  }
}
