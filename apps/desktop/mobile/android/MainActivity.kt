package chat.commy.mobile

import android.content.Intent
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    WakePreviews.noteTap(intent)
  }

  override fun onNewIntent(intent: Intent) {
    WakePreviews.noteTap(intent)
    super.onNewIntent(intent)
  }
}
