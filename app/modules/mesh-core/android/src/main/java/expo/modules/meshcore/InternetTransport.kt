package expo.modules.meshcore

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.Parcel
import android.os.ParcelFileDescriptor
import java.io.BufferedInputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.FileInputStream
import java.io.FileOutputStream

/**
 * Internet route (amendment N1). Meshtrike holds no INTERNET permission
 * (section 18): the separate "MeshGRID" add-on (plugins/link)
 * runs the Nym mixnet client behind a VLESS+REALITY bridge. This class binds
 * its signature-protected service, gets one end of a Unix socket pair, and
 * moves sealed envelopes and route state over it:
 *
 *   frame = u32 length (type + payload) | u8 type | payload
 *   to add-on    0x01 CONFIG JSON, 0x02 SEND (u16 address length, address, envelope)
 *   from add-on  0x81 READY address, 0x82 DOWN reason, 0x83 ENVELOPE,
 *                0x84 MODE "bridge" or "direct"
 *
 * Without the add-on the route simply never comes up and the mesh carries
 * everything.
 */
class InternetTransport(private val context: Context, private val sink: Sink) {
  interface Sink {
    fun onReady(address: String)
    fun onDown()
    fun onEnvelope(bytes: ByteArray)
  }

  companion object {
    const val PLUGIN = "org.meshtrike.link"
    private const val SERVICE = "org.meshtrike.internet.NetService"
    private const val DESCRIPTOR = "org.meshtrike.internet.INet/1"
    private const val OPEN = IBinder.FIRST_CALL_TRANSACTION
    private const val CONFIG: Byte = 0x01
    private const val SEND: Byte = 0x02
    private const val READY: Byte = 0x81.toByte()
    private const val DOWN: Byte = 0x82.toByte()
    private const val ENVELOPE: Byte = 0x83.toByte()
    private const val MODE: Byte = 0x84.toByte()
    private const val MAX_FRAME = 70_000
  }

  private val thread = HandlerThread("mesh-net").apply { start() }
  private val h = Handler(thread.looper)

  /** Last config from the core; re-sent on every new pipe. */
  @Volatile private var config: String? = null
  @Volatile private var enabled = false
  @Volatile private var ready = false
  /** How the add-on is connected right now: "bridge", "direct", or null. */
  @Volatile var mode: String? = null
    private set
  private var binding = false
  private var pipe: ParcelFileDescriptor? = null
  private var out: DataOutputStream? = null
  private val writeLock = Any()

  /** The add-on is installed and signed with our own key. */
  val pluginInstalled: Boolean
    get() = runCatching {
      val pm = context.packageManager
      pm.getPackageInfo(PLUGIN, 0)
      pm.checkSignatures(context.packageName, PLUGIN) == PackageManager.SIGNATURE_MATCH
    }.getOrDefault(false)

  private val connection = object : ServiceConnection {
    override fun onServiceConnected(name: ComponentName, service: IBinder) {
      h.post { openPipe(service) }
    }

    override fun onServiceDisconnected(name: ComponentName) {
      h.post { closePipe() }
    }

    override fun onBindingDied(name: ComponentName) {
      h.post {
        closePipe()
        runCatching { context.unbindService(this) }
        binding = false
        // The add-on restarted itself (e.g. bridge <-> direct): bind again.
        if (enabled) h.postDelayed({ ensureBound() }, 1_000)
      }
    }
  }

  /** From the core: run the route or stop it, with this config. */
  fun configure(on: Boolean, configJson: String) = h.post {
    Diag.log("net: configure on=$on installed=$pluginInstalled bound=$binding")
    enabled = on
    config = org.json.JSONObject(configJson).put("enabled", on).toString()
    if (on) ensureBound()
    sendConfig()
    if (!on) {
      setReady(false)
      if (binding) {
        runCatching { context.unbindService(connection) }
        binding = false
        closePipe()
      }
    }
  }

  /** From the core: one sealed envelope to a mixnet address. */
  fun send(address: String, envelope: ByteArray): Boolean {
    if (!ready) return false
    val a = address.toByteArray(Charsets.UTF_8)
    if (a.isEmpty() || a.size > 1024 || envelope.size > 64 * 1024) return false
    val payload = ByteArray(2 + a.size + envelope.size)
    payload[0] = (a.size shr 8).toByte()
    payload[1] = a.size.toByte()
    System.arraycopy(a, 0, payload, 2, a.size)
    System.arraycopy(envelope, 0, payload, 2 + a.size, envelope.size)
    val ok = write(SEND, payload)
    if (Diag.tracing()) Diag.trace("core sealed envelope ${Diag.fp(envelope)} (${envelope.size} B) for mixnet address $address → handed to the add-on: $ok")
    return ok
  }

  private fun ensureBound() {
    if (binding || !pluginInstalled) return
    val i = Intent().setComponent(ComponentName(PLUGIN, SERVICE))
    binding = runCatching { context.bindService(i, connection, Context.BIND_AUTO_CREATE) }.getOrDefault(false)
    Diag.log("net: bind -> $binding")
  }

  private fun openPipe(b: IBinder) {
    closePipe()
    val data = Parcel.obtain()
    val reply = Parcel.obtain()
    val pfd = try {
      data.writeInterfaceToken(DESCRIPTOR)
      b.transact(OPEN, data, reply, 0)
      reply.readException()
      if (reply.readInt() == 1) ParcelFileDescriptor.CREATOR.createFromParcel(reply) else null
    } catch (e: Exception) {
      null
    } finally {
      data.recycle()
      reply.recycle()
    }
    Diag.log("net: pipe ${if (pfd == null) "refused" else "open"}")
    if (pfd == null) return
    pipe = pfd
    synchronized(writeLock) { out = DataOutputStream(FileOutputStream(pfd.fileDescriptor)) }
    Thread({ readLoop(pfd) }, "mesh-net-read").start()
    sendConfig()
  }

  private fun sendConfig() {
    val c = config ?: return
    write(CONFIG, c.toByteArray(Charsets.UTF_8))
  }

  private fun write(type: Byte, payload: ByteArray): Boolean = synchronized(writeLock) {
    val o = out ?: return false
    runCatching {
      o.writeInt(payload.size + 1)
      o.writeByte(type.toInt())
      o.write(payload)
      o.flush()
      true
    }.getOrDefault(false)
  }

  private fun readLoop(pfd: ParcelFileDescriptor) {
    runCatching {
      val i = DataInputStream(BufferedInputStream(FileInputStream(pfd.fileDescriptor), 64 * 1024))
      while (true) {
        val n = i.readInt()
        if (n < 1 || n > MAX_FRAME) break
        val type = i.readByte()
        val p = ByteArray(n - 1)
        i.readFully(p)
        when (type) {
          READY -> {
            Diag.log("net: route ready")
            setReady(true)
            sink.onReady(String(p, Charsets.UTF_8))
          }
          DOWN -> {
            Diag.log("net: route down (${String(p, Charsets.UTF_8)})")
            mode = null
            setReady(false)
          }
          ENVELOPE -> sink.onEnvelope(p)
          MODE -> {
            mode = String(p, Charsets.UTF_8).takeIf { it == "bridge" || it == "direct" }
            Diag.log("net: route mode $mode")
          }
        }
      }
    }
    h.post { if (pipe === pfd) closePipe() }
  }

  private fun setReady(r: Boolean) {
    val was = ready
    ready = r
    if (was && !r) sink.onDown()
  }

  private fun closePipe() {
    val p = pipe ?: return
    pipe = null
    synchronized(writeLock) { out = null }
    runCatching { android.system.Os.shutdown(p.fileDescriptor, android.system.OsConstants.SHUT_RDWR) }
    runCatching { p.close() }
    setReady(false)
  }
}
