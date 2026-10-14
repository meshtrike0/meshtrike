package expo.modules.meshcore

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.Handler
import android.os.HandlerThread
import androidx.fragment.app.FragmentActivity
import uniffi.mesh_ffi.MeshCore
import uniffi.mesh_ffi.NativeHost
import java.util.ArrayDeque

/**
 * Process-wide owner of the Rust core. All core calls run on one thread
 * ("mesh-core"); BLE and UI calls are posted to it. Native host callbacks
 * coming out of the core never call back in synchronously.
 */
object MeshRuntime {
  @Volatile var core: MeshCore? = null
    private set
  private var thread: HandlerThread? = null
  private var handler: Handler? = null
  private lateinit var keystore: HardwareKeystore
  lateinit var ble: BleTransport
    private set
  lateinit var lora: LoraBridge
    private set
  lateinit var wifi: WifiDirectTransport
    private set
  lateinit var internet: InternetTransport
    private set
  @Volatile var eventListener: ((String) -> Unit)? = null
  @Volatile var foreground = false
  @Volatile private var lastLockedNotify = 0L
  /** Alerts since the user last opened the app; reset in OnActivityEntersForeground. */
  @Volatile var unseen = 0
  private var appContext: Context? = null
  @Volatile var activity: (() -> FragmentActivity?) = { null }
  private var stationary = Stationary()

  @Synchronized
  fun start(context: Context): MeshCore {
    core?.let { return it }
    val app = context.applicationContext
    appContext = app
    val t = HandlerThread("mesh-core").apply { start() }
    val h = Handler(t.looper)
    thread = t
    handler = h
    keystore = HardwareKeystore(app) { activity() }
    ble = BleTransport(app, object : BleTransport.Sink {
      override fun onPeerFound(handle: Long, uuids: List<ByteArray>, rssi: Int) = post { c ->
        val hex = uuids.joinToString("\",\"", "[\"", "\"]") { u -> u.joinToString("") { "%02x".format(it) } }
        c.call("on_peer_found", "{\"peer\":$handle,\"uuids\":$hex,\"rssi\":$rssi,\"transport\":\"Ble\"}")
      }
      override fun onConnected(handle: Long, mtu: Int, outgoing: Boolean) = post { c ->
        c.call("on_connected", "{\"peer\":$handle,\"mtu\":$mtu,\"outgoing\":$outgoing}")
      }
      override fun onFrame(handle: Long, bytes: ByteArray) = post { c -> c.onFrame(handle.toULong(), bytes) }
      override fun onDisconnected(handle: Long) = post { c -> c.call("on_disconnected", "{\"peer\":$handle}") }
    })
    wifi = WifiDirectTransport(app, object : WifiDirectTransport.Sink {
      override fun onConnected(handle: Long, mtu: Int, outgoing: Boolean) = post { c ->
        c.call("on_connected", "{\"peer\":$handle,\"mtu\":$mtu,\"outgoing\":$outgoing,\"transport\":\"WifiP2p\"}")
      }
      override fun onFrame(handle: Long, bytes: ByteArray) = post { c -> c.onFrame(handle.toULong(), bytes) }
      override fun onDisconnected(handle: Long) = post { c -> c.call("on_disconnected", "{\"peer\":$handle}") }
    })
    internet = InternetTransport(app, object : InternetTransport.Sink {
      override fun onReady(address: String) = post { c ->
        c.call("on_internet_ready", org.json.JSONObject().put("address", address).toString())
      }
      override fun onDown() = post { c -> c.call("on_internet_down", "{}") }
      override fun onEnvelope(bytes: ByteArray) = post { c ->
        val r = c.call("on_internet_envelope", "{\"bytes\":\"${android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP)}\"}")
        if (Diag.tracing()) Diag.trace("envelope ${Diag.fp(bytes)} from the add-on → core accepted: $r")
      }
    })
    val dir = app.noBackupFilesDir.absolutePath
    lora = LoraBridge(
      app,
      onEnvelope = { env -> post { c -> c.call("on_lora_packet", "{\"bytes\":\"${android.util.Base64.encodeToString(env, android.util.Base64.NO_WRAP)}\"}") } },
      encode = { env -> core?.loraEncode(env, java.security.SecureRandom().nextInt().toUInt()) },
      decode = { fr -> core?.loraDecode(fr) },
    )
    val c = MeshCore(dir, Host())
    core = c
    lora.restore()
    stationary.register(app)
    // 100 ms tick; 10 s radio sample (section 12.2).
    h.post(object : Runnable {
      var n = 0
      override fun run() {
        core?.tick()
        // Opt-in diagnostics: mirror new core log lines (no identifiers) to logcat.
        if (n % 10 == 0 && Diag.enabled()) mirrorCoreLog()
        if (++n % 100 == 0) {
          val adverts = ble.advertsSeen.getAndSet(0)
          val failed = ble.failedConnects.getAndSet(0)
          core?.call("on_radio_sample", "{\"adverts\":$adverts,\"failed\":$failed,\"stationary\":${stationary.isStationary()}}")
        }
        h.postDelayed(this, 100)
      }
    })
    return c
  }

  private var lastLogTail: List<String> = emptyList()

  private fun mirrorCoreLog() {
    val raw = runCatching { org.json.JSONObject(core?.call("debug_log", "{}") ?: return).optString("ok") }.getOrNull() ?: return
    val lines = raw.split('\n').filter { it.isNotBlank() }
    // Find where the last mirror stopped by its final few lines, not one
    // line: the log repeats lines (same text in the same second).
    val tail = lastLogTail
    val start = if (tail.isEmpty()) (lines.size - 20).coerceAtLeast(0) else {
      var at = -1
      for (i in lines.size - tail.size downTo 0) {
        if (lines.subList(i, i + tail.size) == tail) { at = i + tail.size; break }
      }
      if (at < 0) 0 else at
    }
    for (l in lines.drop(start)) android.util.Log.i("MeshtrikeCore", l)
    if (lines.isNotEmpty()) lastLogTail = lines.takeLast(8)
  }

  fun post(block: (MeshCore) -> Unit) {
    val c = core ?: return
    handler?.post { runCatching { block(c) } }
  }

  /** Microphone audio for the current call (called from the capture thread). */
  fun callSendAudio(bundle: ByteArray) {
    core?.callSendAudio(bundle)
  }

  /** Blocking call from a module background thread. */
  fun call(method: String, args: String): String = core?.call(method, args) ?: "{\"err\":{\"kind\":\"Internal\",\"detail\":1}}"

  fun decodeQr(width: Int, height: Int, luma: ByteArray): List<ByteArray> =
    core?.decodeQr(width.toUInt(), height.toUInt(), luma) ?: emptyList()

  private class Host : NativeHost {
    override fun startAdvertising(serviceUuids: List<ByteArray>, beacon: ByteArray, power: UByte) = ble.startAdvertising(serviceUuids, power.toInt()).let { }
    override fun stopAdvertising() = ble.stopAdvertising().let { }
    override fun startScanning(serviceUuids: List<ByteArray>) = ble.startScanning(serviceUuids, false).let { }
    override fun stopScanning() = ble.stopScanning().let { }
    override fun connect(peer: ULong) = ble.connect(peer.toLong()).let { }
    override fun sendFrame(peer: ULong, bytes: ByteArray): Boolean {
      val h = peer.toLong()
      return if (WifiDirectTransport.isWifiHandle(h)) wifi.sendFrame(h, bytes) else ble.sendFrame(h, bytes)
    }
    override fun disconnect(peer: ULong) {
      val h = peer.toLong()
      if (WifiDirectTransport.isWifiHandle(h)) wifi.disconnect(h) else ble.disconnect(h)
    }
    override fun loraPaired(): Boolean = lora.connected
    override fun wifiP2p(): Boolean = wifi.available
    override fun wifiLink(peer: ULong, groupOwner: Boolean, networkName: String, passphrase: String, authToken: ByteArray) =
      wifi.open(groupOwner, networkName, passphrase, authToken).let { }
    override fun internetConfig(enabled: Boolean, configJson: String) = internet.configure(enabled, configJson).let { }
    override fun internetSend(address: String, envelope: ByteArray): Boolean = internet.send(address, envelope)
    override fun callAudio(data: ByteArray) = CallAudio.onRemote(data)
    override fun loraSend(bytes: ByteArray): Boolean = lora.send(bytes)
    override fun ksWrap(slot: UByte, plaintext: ByteArray): ByteArray? = keystore.wrap(slot.toInt(), plaintext)
    override fun ksUnwrap(slot: UByte, wrapped: ByteArray): ByteArray? = keystore.unwrap(slot.toInt(), wrapped)
    override fun ksHmac(slot: UByte, data: ByteArray): ByteArray? = keystore.hmac(data)
    override fun ksStoreBiometric(secret: ByteArray): Boolean = keystore.storeBiometric(secret)
    override fun ksLoadBiometric(): ByteArray? = keystore.loadBiometric()
    override fun ksCounterGet(): UInt = keystore.counterGet().toUInt()
    override fun ksCounterSet(v: UInt) = keystore.counterSet(v.toInt())
    override fun ksDestroyAll() = keystore.destroyAll()
    override fun emit(eventJson: String) {
      eventListener?.invoke(eventJson)
      // Notifications never contain content or sender names (section 17).
      val ctx = appContext ?: return
      if (eventJson.startsWith("{\"event\":\"CallChanged\"")) {
        CallAudio.onCallEvent(ctx, eventJson, foreground)
        return
      }
      if (eventJson.startsWith("{\"event\":\"MessageAdded\"") && eventJson.contains("\"outgoing\":false") && !foreground) {
        notifyNewMessage(ctx)
      } else if (eventJson == "{\"event\":\"IncomingMessage\"}" && !foreground) {
        val now = System.currentTimeMillis()
        // Locked: content and receipts look alike, so at most one alert per 30 s.
        if (now - lastLockedNotify > 30_000) {
          lastLockedNotify = now
          notifyNewMessage(ctx)
        }
      }
    }
  }

  private fun notifyNewMessage(ctx: Context) {
    val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as android.app.NotificationManager
    if (android.os.Build.VERSION.SDK_INT >= 26) {
      nm.createNotificationChannel(
        android.app.NotificationChannel("mesh-messages", "Messages", android.app.NotificationManager.IMPORTANCE_HIGH).apply {
          lockscreenVisibility = android.app.Notification.VISIBILITY_PRIVATE
        },
      )
    }
    val launch = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName)
    val pi = launch?.let { android.app.PendingIntent.getActivity(ctx, 0, it, android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT) }
    unseen += 1
    val n = androidx.core.app.NotificationCompat.Builder(ctx, "mesh-messages")
      .setContentTitle(if (unseen == 1) "New message" else "$unseen new messages")
      .setNumber(unseen)
      .setOnlyAlertOnce(false)
      .setWhen(System.currentTimeMillis())
      .setShowWhen(true)
      .setSmallIcon(android.R.drawable.stat_notify_chat)
      .setAutoCancel(true)
      .setContentIntent(pi)
      .setVisibility(androidx.core.app.NotificationCompat.VISIBILITY_PRIVATE)
      .setPriority(androidx.core.app.NotificationCompat.PRIORITY_HIGH)
      .setCategory(androidx.core.app.NotificationCompat.CATEGORY_MESSAGE)
      .setDefaults(androidx.core.app.NotificationCompat.DEFAULT_ALL)
      .build()
    runCatching { nm.notify(2, n) }
  }

  /** Accelerometer magnitude variance below 0.05 m/s² over 60 s (12.2). */
  private class Stationary : SensorEventListener {
    private val window = ArrayDeque<Pair<Long, Float>>()

    fun register(context: Context) {
      val sm = context.getSystemService(Context.SENSOR_SERVICE) as SensorManager
      sm.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)?.let { sm.registerListener(this, it, SensorManager.SENSOR_DELAY_NORMAL) }
    }

    @Synchronized
    override fun onSensorChanged(e: SensorEvent) {
      val m = Math.sqrt((e.values[0] * e.values[0] + e.values[1] * e.values[1] + e.values[2] * e.values[2]).toDouble()).toFloat()
      val now = System.currentTimeMillis()
      window.addLast(now to m)
      while (window.isNotEmpty() && now - window.first.first > 60_000) window.removeFirst()
    }

    override fun onAccuracyChanged(s: Sensor?, a: Int) {}

    @Synchronized
    fun isStationary(): Boolean {
      if (window.size < 20 || window.last.first - window.first.first < 55_000) return false
      val mean = window.map { it.second }.average()
      val variance = window.map { (it.second - mean) * (it.second - mean) }.average()
      return variance < 0.05
    }
  }
}
