package chat.commy.mobile

import android.content.Intent
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    WakePreviews.noteTap(this, intent)
  }

  override fun onNewIntent(intent: Intent) {
    WakePreviews.noteTap(this, intent)
    super.onNewIntent(intent)
  }
}
