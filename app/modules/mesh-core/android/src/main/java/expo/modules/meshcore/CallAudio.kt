package expo.modules.meshcore

import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaFormat
import android.media.MediaRecorder
import android.media.Ringtone
import android.media.RingtoneManager
import android.media.ToneGenerator
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
import androidx.core.app.NotificationCompat
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.LinkedBlockingDeque
import java.util.concurrent.TimeUnit

/**
 * Audio for nearby calls.
 *
 * Microphone → Opus (the platform encoder, 20 ms packets) → bundles of four
 * packets (80 ms) → the core, which seals them for the link. Bundles from the
 * other phone → Opus decoder → the earpiece (or speaker), behind a small
 * jitter buffer that drops audio rather than fall behind.
 *
 * Bundle: count u8, then per packet: length u16 (big-endian) | packet.
 */
@SuppressLint("MissingPermission")
internal object CallAudio {
  private const val IN_RATE = 16_000
  private const val FRAME_SAMPLES = IN_RATE / 50 // 20 ms
  private const val PACKETS_PER_BUNDLE = 4
  private const val MAX_QUEUED = 5 // bundles (400 ms) before old audio is dropped
  private const val PRIME = 2 // bundles buffered before playback starts
  private val MUTED = byteArrayOf(0)

  @Volatile private var running = false
  @Volatile var muted = false
  private val incoming = LinkedBlockingDeque<ByteArray>()
  private var capture: Thread? = null
  private var playback: Thread? = null
  private var am: AudioManager? = null

  /** From the core (its thread): queue, never block. */
  fun onRemote(bundle: ByteArray) {
    if (!running) return
    incoming.offerLast(bundle)
    while (incoming.size > MAX_QUEUED) incoming.pollFirst()
  }

  fun start(ctx: Context, wifi: Boolean) {
    if (running) return
    running = true
    muted = false
    incoming.clear()
    val audio = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    am = audio
    audio.mode = AudioManager.MODE_IN_COMMUNICATION
    setSpeaker(ctx, false)
    capture = Thread({ captureLoop(if (wifi) 24_000 else 16_000) }, "call-capture").apply { start() }
    playback = Thread({ playLoop() }, "call-playback").apply { start() }
  }

  fun stop(ctx: Context) {
    running = false
    capture?.interrupt()
    playback?.interrupt()
    capture = null
    playback = null
    incoming.clear()
    am?.let { a ->
      if (Build.VERSION.SDK_INT >= 31) runCatching { a.clearCommunicationDevice() } else @Suppress("DEPRECATION") runCatching { a.isSpeakerphoneOn = false }
      a.mode = AudioManager.MODE_NORMAL
    }
    am = null
  }

  fun setSpeaker(ctx: Context, on: Boolean) {
    val a = am ?: (ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager)
    if (Build.VERSION.SDK_INT >= 31) {
      val want = if (on) AudioDeviceInfo.TYPE_BUILTIN_SPEAKER else AudioDeviceInfo.TYPE_BUILTIN_EARPIECE
      val dev = a.availableCommunicationDevices.firstOrNull { it.type == want }
      if (dev != null) a.setCommunicationDevice(dev) else a.clearCommunicationDevice()
    } else {
      @Suppress("DEPRECATION")
      a.isSpeakerphoneOn = on
    }
  }

  // ===== Microphone → Opus → core =====

  private fun captureLoop(bitrate: Int) {
    val minBuf = AudioRecord.getMinBufferSize(IN_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
    val rec = runCatching {
      AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, IN_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(minBuf, FRAME_SAMPLES * 2 * 8))
    }.getOrNull() ?: return
    val aec = if (AcousticEchoCanceler.isAvailable()) runCatching { AcousticEchoCanceler.create(rec.audioSessionId)?.apply { enabled = true } }.getOrNull() else null
    val ns = if (NoiseSuppressor.isAvailable()) runCatching { NoiseSuppressor.create(rec.audioSessionId)?.apply { enabled = true } }.getOrNull() else null
    val enc = runCatching {
      MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS).apply {
        val f = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, IN_RATE, 1)
        f.setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        configure(f, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        start()
      }
    }.getOrNull()
    if (enc == null) {
      rec.release()
      return
    }
    val pcm = ByteArray(FRAME_SAMPLES * 2)
    val info = MediaCodec.BufferInfo()
    val bundle = ArrayList<ByteArray>(PACKETS_PER_BUNDLE)
    var pts = 0L
    try {
      rec.startRecording()
      while (running && !Thread.currentThread().isInterrupted) {
        var got = 0
        while (got < pcm.size && running) {
          val n = rec.read(pcm, got, pcm.size - got)
          if (n <= 0) break
          got += n
        }
        if (got < pcm.size) continue
        val ii = enc.dequeueInputBuffer(10_000)
        if (ii >= 0) {
          enc.getInputBuffer(ii)?.apply { clear(); put(pcm) }
          enc.queueInputBuffer(ii, 0, pcm.size, pts, 0)
          pts += 20_000
        }
        while (true) {
          val oi = enc.dequeueOutputBuffer(info, 0)
          if (oi < 0) break
          val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
          if (!config && info.size > 0) {
            val buf = enc.getOutputBuffer(oi)
            val pkt = ByteArray(info.size)
            buf?.position(info.offset)
            buf?.get(pkt)
            bundle += pkt
          }
          enc.releaseOutputBuffer(oi, false)
          if (bundle.size >= PACKETS_PER_BUNDLE) {
            // Muted: an empty bundle tells the other phone the call is
            // still there (silence would look like a lost link).
            MeshRuntime.callSendAudio(if (muted) MUTED else pack(bundle))
            bundle.clear()
          }
        }
      }
    } catch (_: Exception) {
    } finally {
      runCatching { rec.stop() }
      rec.release()
      aec?.release()
      ns?.release()
      runCatching { enc.stop() }
      enc.release()
    }
  }

  private fun pack(packets: List<ByteArray>): ByteArray {
    val out = ByteArrayOutputStream()
    out.write(packets.size)
    for (p in packets) {
      out.write(p.size shr 8)
      out.write(p.size and 0xFF)
      out.write(p)
    }
    return out.toByteArray()
  }

  private fun unpack(b: ByteArray): List<ByteArray> {
    if (b.isEmpty()) return emptyList()
    val n = b[0].toInt() and 0xFF
    val out = ArrayList<ByteArray>(n)
    var pos = 1
    repeat(n) {
      if (pos + 2 > b.size) return out
      val len = ((b[pos].toInt() and 0xFF) shl 8) or (b[pos + 1].toInt() and 0xFF)
      pos += 2
      if (len <= 0 || pos + len > b.size) return out
      out += b.copyOfRange(pos, pos + len)
      pos += len
    }
    return out
  }

  // ===== core → Opus → speaker =====

  private fun le(v: Long): ByteBuffer = ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putLong(v).apply { flip() }

  private fun playLoop() {
    val dec = runCatching {
      MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS).apply {
        val f = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, 48_000, 1)
        val head = ByteBuffer.allocate(19).order(ByteOrder.LITTLE_ENDIAN)
        head.put("OpusHead".toByteArray()).put(1).put(1).putShort(0).putInt(IN_RATE).putShort(0).put(0)
        head.flip()
        f.setByteBuffer("csd-0", head)
        f.setByteBuffer("csd-1", le(0))
        f.setByteBuffer("csd-2", le(80_000_000))
        configure(f, null, null, 0)
        start()
      }
    }.getOrNull() ?: return
    var track: AudioTrack? = null
    var rate = 48_000
    val info = MediaCodec.BufferInfo()
    var pts = 0L
    var primed = false
    try {
      while (running && !Thread.currentThread().isInterrupted) {
        if (!primed) {
          if (incoming.size < PRIME) {
            Thread.sleep(20)
            continue
          }
          primed = true
        }
        val bundle = incoming.pollFirst(200, TimeUnit.MILLISECONDS)
        if (bundle == null) {
          primed = false // ran dry: buffer again before resuming
          continue
        }
        for (pkt in unpack(bundle)) {
          val ii = dec.dequeueInputBuffer(10_000)
          if (ii < 0) continue
          dec.getInputBuffer(ii)?.apply { clear(); put(pkt) }
          dec.queueInputBuffer(ii, 0, pkt.size, pts, 0)
          pts += 20_000
          while (true) {
            val oi = dec.dequeueOutputBuffer(info, 0)
            if (oi == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
              rate = dec.outputFormat.getInteger(MediaFormat.KEY_SAMPLE_RATE)
              track?.release()
              track = null
              continue
            }
            if (oi < 0) break
            if (info.size > 0) {
              val t = track ?: newTrack(rate).also { track = it; it.play() }
              val buf = dec.getOutputBuffer(oi)
              val out = ByteArray(info.size)
              buf?.position(info.offset)
              buf?.get(out)
              t.write(out, 0, out.size)
            }
            dec.releaseOutputBuffer(oi, false)
          }
        }
      }
    } catch (_: InterruptedException) {
    } catch (_: Exception) {
    } finally {
      track?.let { runCatching { it.stop() }; it.release() }
      runCatching { dec.stop() }
      dec.release()
    }
  }

  private fun newTrack(rate: Int): AudioTrack {
    val min = AudioTrack.getMinBufferSize(rate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
    return AudioTrack.Builder()
      .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
      .setAudioFormat(AudioFormat.Builder().setSampleRate(rate).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
      .setBufferSizeInBytes(maxOf(min, rate / 5 * 2))
      .setTransferMode(AudioTrack.MODE_STREAM)
      .build()
  }

  // ===== Ringing =====

  private var ringtone: Ringtone? = null
  private var tone: ToneGenerator? = null
  private var vibrator: Vibrator? = null

  /**
   * The caller's ring-back tone ("outgoing") or silence ("none"). Only the
   * tone: the incoming ringtone belongs to the call's own state (onCallEvent),
   * so the call screen opening can never silence it.
   */
  fun ring(ctx: Context, kind: String) {
    tone?.release()
    tone = null
    if (kind == "outgoing") {
      tone = runCatching { ToneGenerator(AudioManager.STREAM_VOICE_CALL, 70).apply { startTone(ToneGenerator.TONE_SUP_RINGTONE) } }.getOrNull()
    }
  }

  private fun startRinging(ctx: Context) {
    if (ringtone != null) return
    val uri = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
    ringtone = RingtoneManager.getRingtone(ctx, uri)?.apply {
      if (Build.VERSION.SDK_INT >= 28) isLooping = true
      audioAttributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build()
      play()
    }
    val v = ctx.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
    v?.vibrate(VibrationEffect.createWaveform(longArrayOf(0, 600, 900), 0))
    vibrator = v
  }

  private fun stopRinging() {
    ringtone?.stop()
    ringtone = null
    vibrator?.cancel()
    vibrator = null
  }

  /** Set by the module: the app's activity, when it has one. */
  @Volatile var activity: (() -> android.app.Activity?)? = null
  @Volatile var ringing = false

  /**
   * While a call rings, the app may show over the lock screen (it then hides
   * who is calling until the phone is unlocked, see the call screen).
   */
  fun showOverLock(on: Boolean) {
    if (Build.VERSION.SDK_INT < 27) return
    val act = activity?.invoke() ?: return
    act.runOnUiThread {
      act.setShowWhenLocked(on)
      act.setTurnScreenOn(on)
    }
  }

  // The ring's full-screen alert may start a fresh activity: it too may
  // show over the lock screen while the call rings.
  private var hooked = false
  private fun hookActivities(ctx: Context) {
    if (hooked || Build.VERSION.SDK_INT < 27) return
    hooked = true
    (ctx.applicationContext as? android.app.Application)?.registerActivityLifecycleCallbacks(object : android.app.Application.ActivityLifecycleCallbacks {
      override fun onActivityCreated(a: android.app.Activity, b: android.os.Bundle?) {
        if (ringing) {
          a.setShowWhenLocked(true)
          a.setTurnScreenOn(true)
        }
      }
      override fun onActivityStarted(a: android.app.Activity) {}
      override fun onActivityResumed(a: android.app.Activity) {}
      override fun onActivityPaused(a: android.app.Activity) {}
      override fun onActivityStopped(a: android.app.Activity) {}
      override fun onActivitySaveInstanceState(a: android.app.Activity, b: android.os.Bundle) {}
      override fun onActivityDestroyed(a: android.app.Activity) {}
    })
  }

  /** The incoming ring and its alert, from core call events (any app state). */
  fun onCallEvent(ctx: Context, json: String, foreground: Boolean) {
    val ringing = json.contains("\"state\":\"Ringing\"") && json.contains("\"outgoing\":false")
    hookActivities(ctx)
    this.ringing = ringing
    showOverLock(ringing)
    if (ringing) {
      startRinging(ctx)
      if (!foreground) notifyIncoming(ctx, true)
    } else {
      stopRinging()
      notifyIncoming(ctx, false)
    }
  }

  /** "Incoming call" over the lock screen. No name: notifications never carry one. */
  fun notifyIncoming(ctx: Context, show: Boolean) {
    val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    if (!show) {
      nm.cancel(3)
      return
    }
    if (Build.VERSION.SDK_INT >= 26) {
      nm.createNotificationChannel(NotificationChannel("mesh-calls", "Calls", NotificationManager.IMPORTANCE_HIGH).apply {
        lockscreenVisibility = android.app.Notification.VISIBILITY_PUBLIC
        setSound(null, null)
      })
    }
    val launch = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName)
    val pi = launch?.let { PendingIntent.getActivity(ctx, 3, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT) }
    val n = NotificationCompat.Builder(ctx, "mesh-calls")
      .setContentTitle("Incoming call")
      .setContentText("Open Meshtrike to answer")
      .setSmallIcon(android.R.drawable.stat_sys_phone_call)
      .setCategory(NotificationCompat.CATEGORY_CALL)
      .setPriority(NotificationCompat.PRIORITY_MAX)
      .setOngoing(true)
      .setContentIntent(pi)
      .setFullScreenIntent(pi, true)
      .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
      .build()
    runCatching { nm.notify(3, n) }
  }
}
