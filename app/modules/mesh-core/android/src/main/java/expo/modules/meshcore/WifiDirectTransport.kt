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
import android.os.SystemClock
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.FileInputStream
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.atomic.AtomicLong

/**
 * Wi-Fi Direct transport (section 9). The core asks for it over an existing
 * BLE link when a large transfer is waiting, and hands both phones the same
 * group name, passphrase and a separate 32-byte auth token, all derived from
 * that link's Noise handshake.
 *
 * Meshtrike holds no INTERNET and no Wi-Fi permission (section 18). The
 * separate "MeshGRID" add-on (plugins/link) creates or joins the group
 * and hands back one end of a Unix socket pair over a signature-protected
 * Binder service; it relays those bytes over TCP inside the group. Both
 * Meshtrike ends then exchange the auth token (which the add-on never sees)
 * before any frame flows. Frames are length-prefixed and already
 * Noise-encrypted. Without the add-on, BLE carries everything.
 */
class WifiDirectTransport(private val context: Context, private val sink: Sink) {
  interface Sink {
    fun onConnected(handle: Long, mtu: Int, outgoing: Boolean)
    fun onFrame(handle: Long, bytes: ByteArray)
    fun onDisconnected(handle: Long)
  }

  companion object {
    /** Wi-Fi link handles never collide with BLE handles (counted from 1). */
    const val HANDLE_BASE = 1L shl 40
    const val MTU = 16_384
    const val PLUGIN = "org.meshtrike.link"
    private const val PLUGIN_SERVICE = "org.meshtrike.wifilink.LinkService"
    private const val PLUGIN_INFO = "org.meshtrike.wifilink.InfoActivity"
    private const val DESCRIPTOR = "org.meshtrike.wifilink.ILink/2"
    private const val OPEN = IBinder.FIRST_CALL_TRANSACTION
    private const val STATUS = IBinder.FIRST_CALL_TRANSACTION + 1
    private const val ROLE_CREATE = 0
    private const val ROLE_JOIN = 1
    const val HAS_HARDWARE = 1
    const val WIFI_ON = 2
    const val PERMITTED = 4
    private const val READY = HAS_HARDWARE or WIFI_ON or PERMITTED
    private const val MAX_FRAME = 70_000
    private const val MAX_QUEUE = 4_096
    private const val SETUP_TIMEOUT_MS = 45_000L
    private const val TOKEN_LEN = 32
    private val POISON = ByteArray(0)

    fun isWifiHandle(h: Long) = h >= HANDLE_BASE
  }

  private val thread = HandlerThread("mesh-wifi").apply { start() }
  private val h = Handler(thread.looper)
  private val nextHandle = AtomicLong(HANDLE_BASE)

  private class Session(val handle: Long, val token: ByteArray) {
    val out = LinkedBlockingQueue<ByteArray>()
    @Volatile var closed = false
    @Volatile var up = false
    @Volatile var pipe: ParcelFileDescriptor? = null
  }

  @Volatile private var session: Session? = null

  // ===== The add-on =====

  @Volatile private var relay: IBinder? = null
  private var binding = false
  private val connection = object : ServiceConnection {
    override fun onServiceConnected(name: ComponentName, service: IBinder) {
      relay = service
      statusAt = 0
    }

    override fun onServiceDisconnected(name: ComponentName) {
      relay = null
    }

    override fun onBindingDied(name: ComponentName) {
      relay = null
      h.post {
        runCatching { context.unbindService(this) }
        binding = false
      }
    }
  }

  /** The MeshGRID add-on is installed and signed with our own key. */
  val pluginInstalled: Boolean
    get() = runCatching {
      val pm = context.packageManager
      pm.getPackageInfo(PLUGIN, 0)
      pm.checkSignatures(context.packageName, PLUGIN) == PackageManager.SIGNATURE_MATCH
    }.getOrDefault(false)

  /** Bind the add-on's service once it is installed (kept bound after that). */
  private fun ensureBound() {
    if (binding || !pluginInstalled) return
    val i = Intent().setComponent(ComponentName(PLUGIN, PLUGIN_SERVICE))
    binding = runCatching { context.bindService(i, connection, Context.BIND_AUTO_CREATE) }.getOrDefault(false)
  }

  init {
    h.post { ensureBound() }
  }

  @Volatile private var cachedStatus = 0
  @Volatile private var statusAt = 0L
  @Volatile private var lastGoodAt = 0L

  /** HAS_HARDWARE | WIFI_ON | PERMITTED as the add-on reports them (0 without it). */
  fun pluginStatus(): Int {
    val now = SystemClock.elapsedRealtime()
    if (statusAt != 0L && now - statusAt < 2_000) return cachedStatus
    statusAt = now
    if (!pluginInstalled) {
      cachedStatus = 0
      return 0
    }
    val b = relay ?: run {
      h.post { ensureBound() }
      // The system may kill and restart the add-on's process at any time;
      // keep the last answer for a while so capabilities do not flicker.
      if (now - lastGoodAt > 30_000) cachedStatus = 0
      return cachedStatus
    }
    val data = Parcel.obtain()
    val reply = Parcel.obtain()
    cachedStatus = try {
      data.writeInterfaceToken(DESCRIPTOR)
      b.transact(STATUS, data, reply, 0)
      reply.readException()
      reply.readInt().also { lastGoodAt = now }
    } catch (e: Exception) {
      if (now - lastGoodAt > 30_000) 0 else cachedStatus
    } finally {
      data.recycle()
      reply.recycle()
    }
    return cachedStatus
  }

  val available: Boolean get() = pluginStatus() == READY

  /** Shows the add-on's screen, which asks for its Wi-Fi permission. */
  fun openPlugin(): Boolean = runCatching {
    context.startActivity(Intent().setComponent(ComponentName(PLUGIN, PLUGIN_INFO)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    statusAt = 0
    true
  }.getOrDefault(false)

  // ===== Setup =====

  /** Called by the core: open a link to the neighbour it negotiated with over BLE. */
  fun open(groupOwner: Boolean, name: String, passphrase: String, token: ByteArray) = h.post {
    if (token.size != TOKEN_LEN || !available) {
      Diag.log("wifi: not opened (add-on status ${pluginStatus()})")
      return@post
    }
    // One link at a time; the core falls back to BLE for this neighbour.
    session?.let { if (!it.closed) return@post }
    val b = relay ?: return@post
    val s = Session(nextHandle.getAndIncrement(), token.copyOf())
    session = s
    h.postDelayed({ if (!s.up) close(s) }, SETUP_TIMEOUT_MS)
    val data = Parcel.obtain()
    val reply = Parcel.obtain()
    val pfd = try {
      data.writeInterfaceToken(DESCRIPTOR)
      data.writeInt(if (groupOwner) ROLE_CREATE else ROLE_JOIN)
      data.writeString(name)
      data.writeString(passphrase)
      data.writeInt(SETUP_TIMEOUT_MS.toInt())
      b.transact(OPEN, data, reply, 0)
      reply.readException()
      if (reply.readInt() == 1) ParcelFileDescriptor.CREATOR.createFromParcel(reply) else null
    } catch (e: Exception) {
      null
    } finally {
      data.recycle()
      reply.recycle()
    }
    if (pfd == null) {
      Diag.log("wifi: add-on refused the link")
      return@post close(s)
    }
    Diag.log("wifi: add-on setting up the group (${if (groupOwner) "owner" else "joiner"})")
    s.pipe = pfd
    Thread({
      // The add-on forms the group and connects in the background; the
      // peer's token arriving means the link is up. The setup deadline
      // closes the pipe if it never does.
      if (handshake(s, pfd)) {
        Diag.log("wifi: link authenticated")
        h.post { attach(s, pfd, outgoing = !groupOwner) }
      } else {
        Diag.log("wifi: link failed before authentication")
        h.post { close(s) }
      }
    }, "mesh-wifi-handshake").start()
  }

  /**
   * Both Meshtrike ends send the core's auth token and check the other's.
   * The add-on only relays these bytes and never had the token, so neither
   * it nor anything else on the group can stand in for the peer.
   */
  private fun handshake(s: Session, pfd: ParcelFileDescriptor): Boolean = runCatching {
    val o = FileOutputStream(pfd.fileDescriptor)
    o.write(s.token)
    o.flush()
    val theirs = ByteArray(TOKEN_LEN)
    DataInputStream(FileInputStream(pfd.fileDescriptor)).readFully(theirs)
    MessageDigest.isEqual(theirs, s.token)
  }.getOrDefault(false)

  private fun attach(s: Session, pfd: ParcelFileDescriptor, outgoing: Boolean) {
    if (s.closed) return
    s.up = true
    // Announce before any frame is read, so the core sees the link first.
    sink.onConnected(s.handle, MTU, outgoing)
    Thread({
      runCatching {
        val o = DataOutputStream(BufferedOutputStream(FileOutputStream(pfd.fileDescriptor), 64 * 1024))
        while (true) {
          val f = s.out.take()
          if (f === POISON) break
          o.writeInt(f.size)
          o.write(f)
          if (s.out.isEmpty()) o.flush()
        }
      }
      h.post { close(s) }
    }, "mesh-wifi-write").start()
    Thread({
      runCatching {
        val i = DataInputStream(BufferedInputStream(FileInputStream(pfd.fileDescriptor), 64 * 1024))
        while (!s.closed) {
          val n = i.readInt()
          if (n <= 0 || n > MAX_FRAME) break
          val buf = ByteArray(n)
          i.readFully(buf)
          sink.onFrame(s.handle, buf)
        }
      }
      h.post { close(s) }
    }, "mesh-wifi-read").start()
  }

  // ===== Data path =====

  fun sendFrame(handle: Long, bytes: ByteArray): Boolean {
    val s = session ?: return false
    if (s.handle != handle || s.closed || !s.up) return false
    if (s.out.size > MAX_QUEUE) {
      h.post { close(s) }
      return false
    }
    s.out.offer(bytes)
    return true
  }

  fun disconnect(handle: Long) = h.post { session?.takeIf { it.handle == handle }?.let { close(it) } }

  /** Closing our end of the pair makes the add-on drop the TCP link and the group. */
  private fun close(s: Session) {
    if (s.closed) return
    s.closed = true
    val wasUp = s.up
    s.up = false
    s.out.clear()
    s.out.offer(POISON)
    // shutdown() wakes the reader and writer threads; close() alone may not.
    s.pipe?.let { p ->
      runCatching { android.system.Os.shutdown(p.fileDescriptor, android.system.OsConstants.SHUT_RDWR) }
      runCatching { p.close() }
    }
    if (session === s) session = null
    if (wasUp) sink.onDisconnected(s.handle)
  }

  fun shutdown() = h.post { session?.let { close(it) } }
}
