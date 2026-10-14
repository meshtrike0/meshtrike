package expo.modules.meshcore

import android.Manifest
import android.content.Intent
import android.os.Build
import android.view.WindowManager
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import expo.modules.interfaces.permissions.Permissions
import expo.modules.kotlin.Promise
import expo.modules.kotlin.functions.Queues
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class MeshCoreModule : Module() {
  private val context get() = requireNotNull(appContext.reactContext)
  private var imeHooked: android.app.Activity? = null

  /** Every text field: no personalised keyboard learning (section 12.3). */
  private fun hookIme(act: android.app.Activity) {
    if (imeHooked === act) return
    imeHooked = act
    act.window?.decorView?.viewTreeObserver?.addOnGlobalFocusChangeListener { _, focused ->
      val et = focused as? android.widget.EditText ?: return@addOnGlobalFocusChangeListener
      val flag = android.view.inputmethod.EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
      if (et.imeOptions and flag == 0) {
        et.imeOptions = et.imeOptions or flag
        (act.getSystemService(android.content.Context.INPUT_METHOD_SERVICE) as android.view.inputmethod.InputMethodManager).restartInput(et)
      }
    }
  }

  override fun definition() = ModuleDefinition {
    Name("MeshCore")

    Events("onCoreEvent", "onVoiceLevel", "onVoicePlayback", "onVoiceLimit")

    OnCreate {
      appContext.currentActivity?.let { act ->
        act.runOnUiThread {
          act.window?.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
          hookIme(act)
        }
      }
      MeshRuntime.eventListener = { json -> sendEvent("onCoreEvent", mapOf("json" to json)) }
      Voice.emit = { name, body -> sendEvent(name, body) }
      MeshRuntime.activity = { appContext.currentActivity as? FragmentActivity }
      CallAudio.activity = { appContext.currentActivity }
    }

    OnDestroy {
      MeshRuntime.eventListener = null
      Voice.emit = null
      Voice.cancel()
      Voice.stopPlayback(notify = false)
    }

    // ===== Voice notes =====

    // Microphone: asked the first time a voice note is recorded.
    AsyncFunction("micPermission") { promise: Promise ->
      Voice.markAskedMic(context)
      Permissions.askForPermissionsWithPermissionsManager(appContext.permissions, promise, Manifest.permission.RECORD_AUDIO)
    }.runOnQueue(Queues.MAIN)

    Function("micGranted") {
      ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED
    }

    AsyncFunction("voiceStart") { Voice.start(context) }
    AsyncFunction("voiceStop") { Voice.stop() }
    AsyncFunction("voiceCancel") { Voice.cancel(); true }
    AsyncFunction("voicePause") { on: Boolean -> Voice.pause(on) }
    AsyncFunction("voiceSend") { conversation: String, optionsJson: String, replyTo: String? -> Voice.send(conversation, optionsJson, replyTo) }
    AsyncFunction("voicePlay") { message: String, fromMs: Int, speed: Double -> Voice.play(context, message, fromMs, speed.toFloat()); true }
    AsyncFunction("voicePausePlayback") { Voice.pause(); true }
    AsyncFunction("voiceResume") { Voice.resume(); true }
    AsyncFunction("voiceSeek") { ms: Int -> Voice.seek(ms); true }
    AsyncFunction("voiceSpeed") { speed: Double -> Voice.setSpeed(speed.toFloat()); true }
    AsyncFunction("voiceStopPlayback") { Voice.stopPlayback(notify = true); true }

    // ===== Nearby calls =====

    AsyncFunction("callAudio") { on: Boolean, wifi: Boolean ->
      if (on) {
        RelayService.setCallActive(context, true)
        CallAudio.start(context, wifi)
      } else {
        CallAudio.stop(context)
        RelayService.setCallActive(context, false)
      }
      true
    }
    AsyncFunction("callMute") { on: Boolean -> CallAudio.muted = on; true }
    AsyncFunction("callSpeaker") { on: Boolean -> CallAudio.setSpeaker(context, on); true }
    AsyncFunction("callRing") { kind: String -> CallAudio.ring(context, kind); true }
    AsyncFunction("callNotify") { show: Boolean -> CallAudio.notifyIncoming(context, show); true }
    Function("appInForeground") { MeshRuntime.foreground }

    // The phone's own lock screen is up (a call can ring over it).
    Function("deviceLocked") {
      (context.getSystemService(android.content.Context.KEYGUARD_SERVICE) as android.app.KeyguardManager).isKeyguardLocked
    }
    // Ask the phone to unlock (fingerprint, face or PIN): resolves true once it is.
    AsyncFunction("unlockDevice") { promise: Promise ->
      val act = appContext.currentActivity
      val km = context.getSystemService(android.content.Context.KEYGUARD_SERVICE) as android.app.KeyguardManager
      if (!km.isKeyguardLocked) return@AsyncFunction promise.resolve(true)
      if (act == null || android.os.Build.VERSION.SDK_INT < 26) return@AsyncFunction promise.resolve(false)
      km.requestDismissKeyguard(act, object : android.app.KeyguardManager.KeyguardDismissCallback() {
        override fun onDismissSucceeded() = promise.resolve(true)
        override fun onDismissCancelled() = promise.resolve(false)
        override fun onDismissError() = promise.resolve(false)
      })
    }.runOnQueue(Queues.MAIN)

    // Meshtrike's page in Android settings (to turn a permission back on).
    Function("openAppSettings") {
      val i = Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.fromParts("package", context.packageName, null))
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      runCatching { context.startActivity(i) }.isSuccess
    }
    // Asked before, refused, and Android will not ask again.
    Function("micBlocked") {
      val act = appContext.currentActivity ?: return@Function false
      val denied = ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) != android.content.pm.PackageManager.PERMISSION_GRANTED
      denied && Voice.askedMic(context) && !androidx.core.app.ActivityCompat.shouldShowRequestPermissionRationale(act, Manifest.permission.RECORD_AUDIO)
    }

    AsyncFunction("videoInfo") { message: String -> MediaShare.videoInfo(message) }
    AsyncFunction("openVideo") { message: String -> MediaShare.openVideo(context, appContext.currentActivity, message) }

    AsyncFunction("start") {
      MeshRuntime.start(context)
      MediaShare.clearShared(context)
      startRelayService()
      true
    }

    AsyncFunction("call") { method: String, args: String ->
      MeshRuntime.call(method, args)
    }

    // A picked file: read and handed to the core here, off the JS thread.
    AsyncFunction("sendFileUri") { conversation: String, uri: String, name: String, optionsJson: String ->
      MediaShare.sendFile(context, conversation, uri, name, optionsJson)
    }

    // A photo saved to the phone's Photos, or offered to the share sheet,
    // as a fresh JPEG from its decoded pixels.
    AsyncFunction("savePhoto") { message: String ->
      MediaShare.savePhoto(context, message)
    }

    AsyncFunction("sharePhoto") { message: String ->
      MediaShare.sharePhoto(context, appContext.currentActivity, message)
    }

    AsyncFunction("requestPermissions") { promise: Promise ->
      val perms = mutableListOf(Manifest.permission.CAMERA)
      if (Build.VERSION.SDK_INT >= 31) {
        perms += listOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_ADVERTISE, Manifest.permission.BLUETOOTH_CONNECT)
      } else {
        perms += Manifest.permission.ACCESS_FINE_LOCATION
      }
      if (Build.VERSION.SDK_INT >= 33) perms += Manifest.permission.POST_NOTIFICATIONS
      Permissions.askForPermissionsWithPermissionsManager(appContext.permissions, promise, *perms.toTypedArray())
    }.runOnQueue(Queues.MAIN)

    // Current grant state per Android permission, without showing any dialog.
    Function("permissionStatus") {
      val perms = mutableListOf(Manifest.permission.CAMERA)
      if (Build.VERSION.SDK_INT >= 31) {
        perms += listOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_ADVERTISE, Manifest.permission.BLUETOOTH_CONNECT)
      } else {
        perms += Manifest.permission.ACCESS_FINE_LOCATION
      }
      if (Build.VERSION.SDK_INT >= 33) perms += Manifest.permission.POST_NOTIFICATIONS
      perms.associateWith {
        androidx.core.content.ContextCompat.checkSelfPermission(context, it) == android.content.pm.PackageManager.PERMISSION_GRANTED
      }
    }

    // Copied text is marked sensitive and cleared after 60 s (section 12.3).
    Function("copySensitive") { text: String ->
      val cm = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
      val clip = android.content.ClipData.newPlainText("", text)
      if (Build.VERSION.SDK_INT >= 33) {
        clip.description.extras = android.os.PersistableBundle().apply { putBoolean(android.content.ClipDescription.EXTRA_IS_SENSITIVE, true) }
      }
      cm.setPrimaryClip(clip)
      android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({
        runCatching {
          if (cm.primaryClip?.getItemAt(0)?.text?.toString() == text) {
            if (Build.VERSION.SDK_INT >= 28) cm.clearPrimaryClip() else cm.setPrimaryClip(android.content.ClipData.newPlainText("", ""))
          }
        }
      }, 60_000)
    }

    // Hardware key attestation chain (leaf first, base64 DER) for the core's
    // offline verifier (section 12.3). A fresh challenge each time.
    AsyncFunction("attestationChain") {
      runCatching {
        val alias = "mesh.attest"
        val ks = java.security.KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        runCatching { ks.deleteEntry(alias) }
        val challenge = ByteArray(32).also { java.security.SecureRandom().nextBytes(it) }
        val spec = android.security.keystore.KeyGenParameterSpec.Builder(alias, android.security.keystore.KeyProperties.PURPOSE_SIGN)
          .setDigests(android.security.keystore.KeyProperties.DIGEST_SHA256)
          .setAlgorithmParameterSpec(java.security.spec.ECGenParameterSpec("secp256r1"))
          .setAttestationChallenge(challenge)
          .build()
        val g = java.security.KeyPairGenerator.getInstance(android.security.keystore.KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        g.initialize(spec)
        g.generateKeyPair()
        val chain = ks.getCertificateChain(alias)?.map { android.util.Base64.encodeToString(it.encoded, android.util.Base64.NO_WRAP) } ?: emptyList()
        ks.deleteEntry(alias)
        chain
      }.getOrDefault(emptyList())
    }

    // Meshtastic LoRa radio (section 9).
    AsyncFunction("loraScan") { promise: Promise ->
      MeshRuntime.lora.scan { list -> promise.resolve(list.map { mapOf("address" to it.first, "name" to it.second) }) }
    }

    AsyncFunction("loraConnect") { address: String -> MeshRuntime.lora.connect(address); true }

    AsyncFunction("loraForget") { MeshRuntime.lora.forget(); true }

    Function("loraStatus") {
      mapOf("connected" to runCatching { MeshRuntime.lora.connected }.getOrDefault(false), "name" to runCatching { MeshRuntime.lora.radioName }.getOrNull())
    }

    Function("bluetoothEnabled") {
      runCatching { MeshRuntime.ble.available }.getOrDefault(false)
    }

    // The MeshGRID add-on (plugins/link): installed with our signature,
    // and what it reports about Wi-Fi Direct (hardware, Wi-Fi on, its permission).
    Function("wifiLinkStatus") {
      val w = runCatching { MeshRuntime.wifi }.getOrNull()
      val installed = w?.pluginInstalled ?: false
      val st = if (installed) w!!.pluginStatus() else 0
      mapOf(
        "installed" to installed,
        "hardware" to (st and WifiDirectTransport.HAS_HARDWARE != 0),
        "wifiOn" to (st and WifiDirectTransport.WIFI_ON != 0),
        "permitted" to (st and WifiDirectTransport.PERMITTED != 0),
      )
    }

    // The MeshGRID add-on (plugins/link), installed with our signature,
    // its version, and how its internet route is connected.
    Function("internetAddonStatus") {
      val installed = runCatching { MeshRuntime.internet.pluginInstalled }.getOrDefault(false)
      mapOf(
        "installed" to installed,
        "mode" to runCatching { MeshRuntime.internet.mode }.getOrNull(),
        "version" to if (installed) runCatching { context.packageManager.getPackageInfo("org.meshtrike.link", 0).versionName }.getOrNull() else null,
      )
    }

    // SHA-256 of the certificate an installed app is signed with (as Android
    // reports it), formatted AB:CD:..., or null when not installed.
    Function("signingFingerprint") { pkg: String ->
      runCatching {
        val info = context.packageManager.getPackageInfo(pkg, android.content.pm.PackageManager.GET_SIGNING_CERTIFICATES)
        val si = info.signingInfo ?: return@runCatching null
        val certs = if (si.hasMultipleSigners()) si.apkContentsSigners else si.signingCertificateHistory
        val cert = certs?.lastOrNull()?.toByteArray() ?: return@runCatching null
        java.security.MessageDigest.getInstance("SHA-256").digest(cert).joinToString(":") { "%02X".format(it) }
      }.getOrNull()
    }

    // Whether the "Get MeshGRID" prompt was already offered (shown once, unprompted).
    Function("reachPromptSeen") {
      context.getSharedPreferences("meshtrike-ui", android.content.Context.MODE_PRIVATE).getBoolean("reachPrompt", false)
    }
    Function("setReachPromptSeen") {
      context.getSharedPreferences("meshtrike-ui", android.content.Context.MODE_PRIVATE).edit().putBoolean("reachPrompt", true).apply()
      true
    }

    // Opens the add-on's own screen, where it asks for its Wi-Fi permission.
    Function("openWifiLink") {
      runCatching { MeshRuntime.wifi.openPlugin() }.getOrDefault(false)
    }
    // Screenshots, screen recording and the recent-apps thumbnail are blocked
    // on every window (section 12.3).
    OnActivityEntersForeground {
      MeshRuntime.foreground = true
      MeshRuntime.unseen = 0
      // The user is looking at the app: clear the "New message" notification.
      runCatching { androidx.core.app.NotificationManagerCompat.from(context).cancel(2) }
      // May be delivered on the JS thread; window flags must change on the UI thread.
      appContext.currentActivity?.let { act ->
        act.runOnUiThread {
          act.window?.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
          hookIme(act)
        }
      }
      MeshRuntime.post { it.call("set_foreground", "{\"foreground\":true}") }
    }

    OnActivityEntersBackground {
      MeshRuntime.foreground = false
      MeshRuntime.post { it.call("set_foreground", "{\"foreground\":false}") }
    }

    View(MeshCoreView::class) {
      Prop("rgbaBase64") { view: MeshCoreView, v: String -> view.setRgba(v) }
      Prop("pixelWidth") { view: MeshCoreView, v: Int -> view.setPixelWidth(v) }
      Prop("pixelHeight") { view: MeshCoreView, v: Int -> view.setPixelHeight(v) }
    }
  }

  private fun startRelayService() {
    val i = Intent(context, RelayService::class.java)
    runCatching { ContextCompat.startForegroundService(context, i) }
  }
}
