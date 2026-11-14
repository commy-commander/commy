package chat.commy.mobile

import com.google.crypto.tink.subtle.Ed25519Verify
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.security.GeneralSecurityException

object PreviewSignature {
  const val KEY_BYTES = 32
  const val SIGNATURE_BYTES = 64
  private const val CALL = 1
  private const val ENDED = 2
  private val DOMAIN = "commy-wake-preview-signed/v1".toByteArray(Charsets.UTF_8)

  fun signedBytes(key: ByteArray, author: String, channel: String, text: String, call: Boolean, ended: Boolean): ByteArray {
    val out = ByteArrayOutputStream()
    out.write(DOMAIN)
    out.write(key)
    for (field in listOf(author, channel, text)) {
      val bytes = field.toByteArray(Charsets.UTF_8)
      out.write(ByteBuffer.allocate(4).putInt(bytes.size).array())
      out.write(bytes)
    }
    out.write((if (call) CALL else 0) or (if (ended) ENDED else 0))
    return out.toByteArray()
  }

  fun verified(signer: ByteArray?, signature: ByteArray?, signed: ByteArray): Boolean {
    if (signer == null || signature == null || signer.size != KEY_BYTES || signature.size != SIGNATURE_BYTES) return false
    return try {
      Ed25519Verify(signer).verify(signature, signed)
      true
    } catch (e: GeneralSecurityException) {
      false
    }
  }
}
