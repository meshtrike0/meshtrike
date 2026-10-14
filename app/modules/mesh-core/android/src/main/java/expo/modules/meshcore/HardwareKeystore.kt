package expo.modules.meshcore

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import java.io.File
import java.nio.ByteBuffer
import java.security.KeyStore
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.Mac
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Hardware-held keys (section 10, D23, D24): AES-256-GCM wrapping keys and an
 * HMAC-SHA256 key generated in StrongBox, falling back to the TEE. Wrapped
 * blobs are IV (12) ‖ ciphertext+tag. Keys never leave secure hardware.
 */
class HardwareKeystore(private val context: Context, private val activity: () -> FragmentActivity?) {
  private val ks: KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
  private val dir = context.noBackupFilesDir

  private fun alias(slot: Int) = when (slot) {
    0 -> "mesh.relaydb"
    1 -> "mesh.msgdb"
    else -> "mesh.duress"
  }

  private val strongBox: Boolean =
    Build.VERSION.SDK_INT >= 28 && context.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)

  private fun aesKey(slot: Int): SecretKey {
    val a = alias(slot)
    (ks.getKey(a, null) as? SecretKey)?.let { return it }
    fun gen(sb: Boolean): SecretKey {
      val b = KeyGenParameterSpec.Builder(a, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
      // Message-store key usable only while the device is unlocked (section 10).
      if (slot == 1 && Build.VERSION.SDK_INT >= 28) b.setUnlockedDeviceRequired(true)
      if (sb && Build.VERSION.SDK_INT >= 28) b.setIsStrongBoxBacked(true)
      val g = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
      g.init(b.build())
      return g.generateKey()
    }
    return try {
      gen(strongBox)
    } catch (e: StrongBoxUnavailableException) {
      gen(false)
    }
  }

  private fun hmacKey(): SecretKey {
    val a = alias(2)
    (ks.getKey(a, null) as? SecretKey)?.let { return it }
    fun gen(sb: Boolean): SecretKey {
      val b = KeyGenParameterSpec.Builder(a, KeyProperties.PURPOSE_SIGN)
      if (sb && Build.VERSION.SDK_INT >= 28) b.setIsStrongBoxBacked(true)
      val g = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_HMAC_SHA256, "AndroidKeyStore")
      g.init(b.build())
      return g.generateKey()
    }
    return try {
      gen(strongBox)
    } catch (e: Exception) {
      gen(false)
    }
  }

  fun wrap(slot: Int, plaintext: ByteArray): ByteArray? = runCatching {
    val c = Cipher.getInstance("AES/GCM/NoPadding")
    c.init(Cipher.ENCRYPT_MODE, aesKey(slot))
    val ct = c.doFinal(plaintext)
    ByteBuffer.allocate(c.iv.size + ct.size).put(c.iv).put(ct).array()
  }.getOrNull()

  fun unwrap(slot: Int, wrapped: ByteArray): ByteArray? = runCatching {
    if (wrapped.size < 12 + 16) return null
    val c = Cipher.getInstance("AES/GCM/NoPadding")
    c.init(Cipher.DECRYPT_MODE, aesKey(slot), GCMParameterSpec(128, wrapped, 0, 12))
    c.doFinal(wrapped, 12, wrapped.size - 12)
  }.getOrNull()

  fun hmac(data: ByteArray): ByteArray? = runCatching {
    val m = Mac.getInstance("HmacSHA256")
    m.init(hmacKey())
    m.doFinal(data)
  }.getOrNull()

  // ===== Biometric (optional, section 12.3) =====

  private val bioAlias = "mesh.biometric"
  private val bioFile get() = File(dir, "bio.bin")

  private fun bioKey(create: Boolean): SecretKey? {
    (ks.getKey(bioAlias, null) as? SecretKey)?.let { return it }
    if (!create) return null
    val b = KeyGenParameterSpec.Builder(bioAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
      .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
      .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
      .setKeySize(256)
      .setUserAuthenticationRequired(true)
      .setInvalidatedByBiometricEnrollment(true)
    if (Build.VERSION.SDK_INT >= 30) b.setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
    val g = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    g.init(b.build())
    return g.generateKey()
  }

  /** Runs a BiometricPrompt with a CryptoObject; blocks the calling (core) thread. */
  private fun authenticate(cipher: Cipher, title: String): Cipher? {
    val act = activity() ?: return null
    if (BiometricManager.from(context).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) != BiometricManager.BIOMETRIC_SUCCESS) return null
    val latch = CountDownLatch(1)
    var result: Cipher? = null
    Handler(Looper.getMainLooper()).post {
      val prompt = BiometricPrompt(act, ContextCompat.getMainExecutor(context), object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(r: BiometricPrompt.AuthenticationResult) {
          result = r.cryptoObject?.cipher
          latch.countDown()
        }
        override fun onAuthenticationError(code: Int, msg: CharSequence) {
          latch.countDown()
        }
      })
      val info = BiometricPrompt.PromptInfo.Builder()
        .setTitle(title)
        .setNegativeButtonText("Use PIN")
        .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
        .build()
      prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
    }
    latch.await(60, TimeUnit.SECONDS)
    return result
  }

  fun storeBiometric(secret: ByteArray): Boolean = runCatching {
    val c = Cipher.getInstance("AES/GCM/NoPadding")
    c.init(Cipher.ENCRYPT_MODE, bioKey(true))
    val ok = authenticate(c, "Enable fingerprint unlock") ?: return false
    val ct = ok.doFinal(secret)
    bioFile.writeBytes(ok.iv + ct)
    true
  }.getOrDefault(false)

  fun loadBiometric(): ByteArray? = runCatching {
    val blob = bioFile.takeIf { it.exists() }?.readBytes() ?: return null
    val c = Cipher.getInstance("AES/GCM/NoPadding")
    c.init(Cipher.DECRYPT_MODE, bioKey(false) ?: return null, GCMParameterSpec(128, blob, 0, 12))
    val ok = authenticate(c, "Unlock Meshtrike") ?: return null
    ok.doFinal(blob, 12, blob.size - 12)
  }.getOrNull()

  // ===== Wrong-PIN counter =====

  private val counterFile get() = File(dir, "pin_attempts")

  fun counterGet(): Int = runCatching { counterFile.readText().trim().toInt() }.getOrDefault(0)

  fun counterSet(v: Int) {
    runCatching {
      val tmp = File(dir, "pin_attempts.tmp")
      tmp.writeText(v.toString())
      tmp.renameTo(counterFile)
    }
  }

  /** Panic wipe: destroying the hardware keys makes leftover data unreadable. */
  fun destroyAll() {
    for (a in listOf(alias(0), alias(1), alias(2), bioAlias)) runCatching { ks.deleteEntry(a) }
    bioFile.delete()
    counterFile.delete()
  }
}
