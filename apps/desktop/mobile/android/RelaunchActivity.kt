package chat.commy.mobile

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.Process

class RelaunchActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val old = intent.getIntExtra(OLD_PID, -1)
    if (old > 0 && old != Process.myPid()) Process.killProcess(old)
    startActivity(
      Intent(this, MainActivity::class.java)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
    )
    finish()
    Runtime.getRuntime().exit(0)
  }

  companion object {
    const val OLD_PID = "chat.commy.mobile.OLD_PID"
  }
}
