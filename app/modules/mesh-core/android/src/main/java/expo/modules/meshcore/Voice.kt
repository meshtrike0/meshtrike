package expo.modules.meshcore

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaDataSource
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.os.Handler
import android.os.Looper
import android.util.Base64
import org.json.JSONObject
import java.io.File

/**
 * Voice notes: recording with the platform's Opus encoder, and playback of
 * Ogg files the core rebuilt from checked packets.
 *
 * - A recording lives only in the app's private cache while it is made, and
 *   is deleted once sent or cancelled. The core keeps just the Opus packets.
 * - Playback is fed from memory (no file of the other phone's audio is ever
 *   written), and Android decodes it in its separate media process.
 * - Levels and playback position are reported as events for the UI.
 */
internal object Voice {
  private const val MAX_MS = 300_000L
  private const val BARS = 64
  private val main = Handler(Looper.getMainLooper())

  /** Sink for events: (name, payload). Set by the module. */
  @Volatile var emit: ((String, Map<String, Any?>) -> Unit)? = null

  // ===== Recording =====

  /** Whether Android has been asked for the microphone before (to tell "not yet asked" from "blocked"). */
  fun askedMic(ctx: Context): Boolean = ctx.getSharedPreferences("meshtrike-voice", Context.MODE_PRIVATE).getBoolean("askedMic", false)

  fun markAskedMic(ctx: Context) {
    ctx.getSharedPreferences("meshtrike-voice", Context.MODE_PRIVATE).edit().putBoolean("askedMic", true).apply()
  }

  private var recorder: MediaRecorder? = null
  private var file: File? = null
  private var startedAt = 0L
  private var stoppedMs = 0L
  // Time spent paused is not part of the recording.
  private var pausedAt = 0L
  private var pausedTotal = 0L

  private fun elapsed(): Long =
    (if (pausedAt > 0) pausedAt else System.currentTimeMillis()) - startedAt - pausedTotal
  private val levels = ArrayList<Int>()
  private var ticker: Runnable? = null

  fun start(ctx: Context): Boolean {
    val t0 = System.currentTimeMillis()
    cancel()
    val dir = File(ctx.cacheDir, "voice").apply { mkdirs() }
    dir.listFiles()?.forEach { it.delete() }
    val f = File(dir, "rec-${System.nanoTime()}.ogg")
    val r = if (android.os.Build.VERSION.SDK_INT >= 31) MediaRecorder(ctx) else @Suppress("DEPRECATION") MediaRecorder()
    try {
      r.setAudioSource(MediaRecorder.AudioSource.VOICE_COMMUNICATION)
      r.setOutputFormat(MediaRecorder.OutputFormat.OGG)
      r.setAudioEncoder(MediaRecorder.AudioEncoder.OPUS)
      r.setAudioChannels(1)
      r.setAudioSamplingRate(48_000)
      r.setAudioEncodingBitRate(16_000)
      r.setOutputFile(f.absolutePath)
      r.prepare()
      r.start()
    } catch (e: Exception) {
      runCatching { r.release() }
      f.delete()
      throw IllegalStateException("Could not start the microphone")
    }
    recorder = r
    file = f
    startedAt = System.currentTimeMillis()
    Diag.log("voice: microphone started in ${startedAt - t0} ms")
    stoppedMs = 0
    pausedAt = 0
    pausedTotal = 0
    levels.clear()
    val t = object : Runnable {
      override fun run() {
        val rec = recorder ?: return
        if (pausedAt > 0) {
          main.postDelayed(this, 80)
          return
        }
        val amp = runCatching { rec.maxAmplitude }.getOrDefault(0)
        // Perceptual scale: quiet speech still shows.
        val level = (Math.sqrt(amp / 32767.0) * 255).toInt().coerceIn(0, 255)
        levels += level
        val elapsed = elapsed()
        emit?.invoke("onVoiceLevel", mapOf("level" to level, "elapsedMs" to elapsed))
        if (elapsed >= MAX_MS) {
          stop()
          emit?.invoke("onVoiceLimit", mapOf("elapsedMs" to elapsed))
          return
        }
        main.postDelayed(this, 80)
      }
    }
    ticker = t
    main.post(t)
    return true
  }

  /** Pause or resume the recording (the microphone is released meanwhile by the encoder). */
  fun pause(on: Boolean): Long {
    val r = recorder ?: throw IllegalStateException("Not recording")
    if (on && pausedAt == 0L) {
      r.pause()
      pausedAt = System.currentTimeMillis()
    } else if (!on && pausedAt > 0L) {
      r.resume()
      pausedTotal += System.currentTimeMillis() - pausedAt
      pausedAt = 0
    }
    return elapsed()
  }

  /** Stop recording and keep it for sending: its length and waveform. */
  fun stop(): Map<String, Any> {
    ticker?.let { main.removeCallbacks(it) }
    ticker = null
    val r = recorder
    recorder = null
    if (r != null) {
      stoppedMs = elapsed()
      pausedAt = 0
      runCatching { r.stop() }.onFailure { file?.delete(); file = null }
      runCatching { r.release() }
    }
    return mapOf("durationMs" to stoppedMs, "waveform" to waveform(), "ok" to (file?.exists() == true))
  }

  fun cancel() {
    ticker?.let { main.removeCallbacks(it) }
    ticker = null
    recorder?.let { r -> runCatching { r.stop() }; runCatching { r.release() } }
    recorder = null
    file?.delete()
    file = null
    levels.clear()
  }

  /** The kept recording to the core, then gone from the cache. */
  fun send(conversation: String, optionsJson: String, replyTo: String?): String {
    if (recorder != null) stop()
    val f = file ?: throw IllegalStateException("Nothing recorded")
    try {
      val ogg = f.readBytes()
      val args = JSONObject()
        .put("conversation", conversation)
        .put("ogg", Base64.encodeToString(ogg, Base64.NO_WRAP))
        .put("waveform", Base64.encodeToString(waveform().map { it.toByte() }.toByteArray(), Base64.NO_WRAP))
        .put("options", JSONObject(optionsJson))
      if (!replyTo.isNullOrEmpty()) args.put("reply_to", replyTo)
      return MeshRuntime.call("send_voice", args.toString())
    } finally {
      f.delete()
      file = null
      levels.clear()
    }
  }

  /** Levels squeezed into BARS bars (loudest per slot). */
  private fun waveform(): List<Int> {
    if (levels.isEmpty()) return emptyList()
    val out = ArrayList<Int>(BARS)
    for (i in 0 until BARS) {
      val a = i * levels.size / BARS
      val b = maxOf(a + 1, (i + 1) * levels.size / BARS)
      out += levels.subList(a, minOf(b, levels.size)).maxOrNull() ?: 0
    }
    return out
  }

  // ===== Playback =====

  private var player: MediaPlayer? = null
  private var playing: String? = null // message id, or "draft"
  private var poller: Runnable? = null
  private var focus: AudioFocusRequest? = null
  private var audio: AudioManager? = null

  /** Play a voice message from the core (or the unsent recording). */
  fun play(ctx: Context, message: String, fromMs: Int, speed: Float) {
    stopPlayback(notify = true)
    val bytes: ByteArray = if (message == "draft") {
      file?.readBytes() ?: throw IllegalStateException("Nothing recorded")
    } else {
      val reply = JSONObject(MeshRuntime.call("voice_audio", JSONObject().put("message", message).toString()))
      val ok = reply.optJSONObject("ok") ?: throw IllegalStateException("Voice message not available")
      Base64.decode(ok.getString("ogg"), Base64.DEFAULT)
    }
    val p = MediaPlayer()
    val attrs = AudioAttributes.Builder()
      .setUsage(AudioAttributes.USAGE_MEDIA)
      .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
      .build()
    p.setAudioAttributes(attrs)
    p.setDataSource(object : MediaDataSource() {
      override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
        if (position >= bytes.size) return -1
        val n = minOf(size.toLong(), bytes.size - position).toInt()
        System.arraycopy(bytes, position.toInt(), buffer, offset, n)
        return n
      }
      override fun getSize(): Long = bytes.size.toLong()
      override fun close() {}
    })
    p.setOnCompletionListener {
      val id = playing
      stopPlayback(notify = false)
      emit?.invoke("onVoicePlayback", mapOf("message" to id, "positionMs" to 0, "durationMs" to 0, "playing" to false, "ended" to true))
    }
    p.prepare()
    runCatching { p.playbackParams = p.playbackParams.setSpeed(speed) }
    if (fromMs > 0) p.seekTo(fromMs)
    val am = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    val req = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT).setAudioAttributes(attrs).build()
    am.requestAudioFocus(req)
    focus = req
    audio = am
    p.start()
    player = p
    playing = message
    val t = object : Runnable {
      override fun run() {
        val pl = player ?: return
        emit?.invoke("onVoicePlayback", mapOf(
          "message" to playing, "positionMs" to pl.currentPosition, "durationMs" to pl.duration,
          "playing" to pl.isPlaying, "ended" to false,
        ))
        main.postDelayed(this, 100)
      }
    }
    poller = t
    main.post(t)
  }

  fun pause() {
    val p = player ?: return
    runCatching { p.pause() }
    emit?.invoke("onVoicePlayback", mapOf("message" to playing, "positionMs" to p.currentPosition, "durationMs" to p.duration, "playing" to false, "ended" to false))
  }

  fun resume() {
    runCatching { player?.start() }
  }

  fun seek(ms: Int) {
    runCatching { player?.seekTo(ms) }
  }

  fun setSpeed(speed: Float) {
    val p = player ?: return
    val was = p.isPlaying
    runCatching { p.playbackParams = p.playbackParams.setSpeed(speed) }
    // Setting a speed starts a paused player; keep it paused.
    if (!was) runCatching { p.pause() }
  }

  fun stopPlayback(notify: Boolean) {
    poller?.let { main.removeCallbacks(it) }
    poller = null
    val id = playing
    player?.let { p -> runCatching { p.stop() }; runCatching { p.release() } }
    player = null
    playing = null
    focus?.let { f -> audio?.abandonAudioFocusRequest(f) }
    focus = null
    if (notify && id != null) {
      emit?.invoke("onVoicePlayback", mapOf("message" to id, "positionMs" to 0, "durationMs" to 0, "playing" to false, "ended" to false))
    }
  }
}
