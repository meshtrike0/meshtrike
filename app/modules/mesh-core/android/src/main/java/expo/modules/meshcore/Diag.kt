package expo.modules.meshcore

import android.os.SystemClock

/**
 * Opt-in diagnostics for field debugging. Off unless switched on over adb:
 *
 *   adb shell setprop debug.meshtrike.log 1
 *
 * Only an attached computer can set that property, so a normal user never
 * logs anything. Lines carry link handles, sizes and states: no addresses,
 * names, keys or content (section 19).
 */
internal object Diag {
  @Volatile private var on = false
  @Volatile private var checkedAt = 0L

  fun enabled(): Boolean {
    val now = SystemClock.elapsedRealtime()
    if (now - checkedAt > 5_000) {
      checkedAt = now
      on = runCatching {
        Class.forName("android.os.SystemProperties")
          .getMethod("get", String::class.java, String::class.java)
          .invoke(null, "debug.meshtrike.log", "0") == "1"
      }.getOrDefault(false)
    }
    return on
  }

  fun log(msg: String) {
    if (enabled()) android.util.Log.i("Meshtrike", msg)
  }

  // ===== Message trace (separate switch) =====
  //
  //   adb shell setprop debug.meshtrike.trace 1
  //
  // For testing the internet route only: unlike the lines above it names
  // mixnet addresses, so it has its own switch, off unless set over adb.
  // Envelopes appear as a short fingerprint (first bytes of SHA-256 over the
  // sealed envelope), identical on both phones; content is never logged.

  @Volatile private var traceOn = false
  @Volatile private var traceCheckedAt = 0L

  fun tracing(): Boolean {
    val now = SystemClock.elapsedRealtime()
    if (now - traceCheckedAt > 5_000) {
      traceCheckedAt = now
      traceOn = runCatching {
        Class.forName("android.os.SystemProperties")
          .getMethod("get", String::class.java, String::class.java)
          .invoke(null, "debug.meshtrike.trace", "0") == "1"
      }.getOrDefault(false)
    }
    return traceOn
  }

  fun trace(msg: String) {
    if (tracing()) android.util.Log.i("MeshtrikeTrace", "app: $msg")
  }

  fun fp(b: ByteArray): String {
    val d = java.security.MessageDigest.getInstance("SHA-256").digest(b)
    return "#" + d.take(4).joinToString("") { "%02x".format(it) }
  }
}
