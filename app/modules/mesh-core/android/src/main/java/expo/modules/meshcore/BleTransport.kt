package expo.modules.meshcore

import android.annotation.SuppressLint
import android.bluetooth.*
import android.bluetooth.le.*
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelUuid
import java.nio.ByteBuffer
import java.util.UUID
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/**
 * BLE transport (section 9): advertising, scanning, and a GATT link in both
 * roles. Frames are opaque bytes from the core (already Noise-encrypted).
 * Writes and notifications are serialised per link, as Android requires.
 */
@SuppressLint("MissingPermission")
class BleTransport(private val context: Context, private val sink: Sink) {
  interface Sink {
    fun onPeerFound(handle: Long, uuids: List<ByteArray>, rssi: Int)
    fun onConnected(handle: Long, mtu: Int, outgoing: Boolean)
    fun onFrame(handle: Long, bytes: ByteArray)
    fun onDisconnected(handle: Long)
  }

  companion object {
    // Fixed GATT data service. Only visible after connecting; advertising
    // carries the mode-dependent UUIDs chosen by the core (section 5).
    val DATA_SERVICE: UUID = UUID.fromString("6e4d0002-b5a3-4f39-9c2e-0d5a9f1c7b21")
    val RX: UUID = UUID.fromString("6e4d0003-b5a3-4f39-9c2e-0d5a9f1c7b21") // central → peripheral (write)
    val TX: UUID = UUID.fromString("6e4d0004-b5a3-4f39-9c2e-0d5a9f1c7b21") // peripheral → central (notify)
    val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

    fun toUuid(b: ByteArray): UUID {
      val bb = ByteBuffer.wrap(b)
      return UUID(bb.long, bb.long)
    }

    fun toBytes(u: UUID): ByteArray = ByteBuffer.allocate(16).putLong(u.mostSignificantBits).putLong(u.leastSignificantBits).array()
  }

  private val thread = HandlerThread("mesh-ble").apply { start() }
  // A missing permission or a stack hiccup must never kill the radio thread.
  private val ble = object : Handler(thread.looper) {
    override fun dispatchMessage(msg: android.os.Message) {
      try {
        super.dispatchMessage(msg)
      } catch (e: SecurityException) {
      } catch (e: IllegalStateException) {
      } catch (e: IllegalArgumentException) {
      }
    }
  }
  private val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager
  private val adapter: BluetoothAdapter? get() = manager.adapter
  private var server: BluetoothGattServer? = null
  private var txChar: BluetoothGattCharacteristic? = null
  private val nextHandle = AtomicLong(1)

  private class Link(val handle: Long, val device: BluetoothDevice, val central: Boolean) {
    var gatt: BluetoothGatt? = null
    var rx: BluetoothGattCharacteristic? = null
    var mtu = 23
    val queue = ArrayDeque<ByteArray>()
    var busy = false
    var up = false
    // Frames a central writes before we announce the link (we wait for its
    // MTU exchange); replayed in order right after onConnected.
    val early = ArrayDeque<ByteArray>()
  }

  private val links = HashMap<Long, Link>() // guarded by `ble` thread
  private val scanHandles = java.util.concurrent.ConcurrentHashMap<String, Long>() // address → handle from scanning
  private var advertising: AdvertiseCallback? = null
  private var scanning: ScanCallback? = null
  // What the core last asked the radio to do, replayed when Bluetooth comes
  // back on (switching it off drops the GATT server, adverts and scans).
  private var wantAdvert: Pair<List<ByteArray>, Int>? = null
  private var wantScan: Pair<List<ByteArray>, Boolean>? = null

  private val radioState = object : BroadcastReceiver() {
    override fun onReceive(c: Context, i: Intent) {
      when (i.getIntExtra(BluetoothAdapter.EXTRA_STATE, -1)) {
        BluetoothAdapter.STATE_TURNING_OFF, BluetoothAdapter.STATE_OFF -> ble.post { radioOff() }
        BluetoothAdapter.STATE_ON -> ble.post { radioOn() }
      }
    }
  }

  init {
    val f = IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED)
    if (Build.VERSION.SDK_INT >= 33) context.registerReceiver(radioState, f, Context.RECEIVER_NOT_EXPORTED)
    else context.registerReceiver(radioState, f)
  }

  /** Bluetooth went off: every link, the server and our adverts are gone. */
  private fun radioOff() {
    Diag.log("ble: bluetooth off, links and server dropped")
    for (l in links.values.toList()) {
      runCatching { l.gatt?.close() }
      l.gatt = null
      closeLink(l, notify = true)
    }
    links.clear()
    scanHandles.clear()
    serverLinks.clear()
    runCatching { server?.close() }
    server = null
    txChar = null
    serviceReady = false
    advertising = null
    scanning = null
  }

  /** Bluetooth is back: a fresh server, then the core's adverts and scan. */
  private fun radioOn() {
    Diag.log("ble: bluetooth on, restarting")
    serverRetries = 0
    ensureServer()
    wantAdvert?.let { (u, p) -> startAdvertisingNow(u, p) }
    wantScan?.let { (u, low) -> startScanningNow(u, low) }
  }

  val advertsSeen = AtomicInteger(0)
  val failedConnects = AtomicInteger(0)

  val available: Boolean get() = adapter?.isEnabled == true

  // ===== Advertising =====

  fun startAdvertising(uuids: List<ByteArray>, power: Int) = ble.post {
    wantAdvert = uuids to power
    startAdvertisingNow(uuids, power)
  }

  private fun startAdvertisingNow(uuids: List<ByteArray>, power: Int) {
    stopAdvertisingNow()
    val adv = adapter?.bluetoothLeAdvertiser ?: return
    ensureServer()
    val txPower = when (power) {
      0 -> AdvertiseSettings.ADVERTISE_TX_POWER_ULTRA_LOW
      1 -> AdvertiseSettings.ADVERTISE_TX_POWER_LOW
      2 -> AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM
      else -> AdvertiseSettings.ADVERTISE_TX_POWER_HIGH
    }
    val settings = AdvertiseSettings.Builder()
      .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
      .setTxPowerLevel(txPower)
      .setConnectable(true)
      .build()
    // No name, no identifiers: only service UUIDs (section 5). One 128-bit
    // UUID fits the 31-byte advert; a second goes in the scan response.
    val data = AdvertiseData.Builder().setIncludeDeviceName(false).setIncludeTxPowerLevel(false)
    val resp = AdvertiseData.Builder().setIncludeDeviceName(false)
    uuids.forEachIndexed { i, u ->
      if (i == 0) data.addServiceUuid(ParcelUuid(toUuid(u))) else if (i == 1) resp.addServiceUuid(ParcelUuid(toUuid(u)))
    }
    val cb = object : AdvertiseCallback() {}
    advertising = cb
    runCatching { adv.startAdvertising(settings, data.build(), resp.build(), cb) }
  }

  fun stopAdvertising() = ble.post {
    wantAdvert = null
    stopAdvertisingNow()
  }

  private fun stopAdvertisingNow() {
    advertising?.let { cb -> runCatching { adapter?.bluetoothLeAdvertiser?.stopAdvertising(cb) } }
    advertising = null
  }

  // ===== Scanning =====

  fun startScanning(uuids: List<ByteArray>, lowPower: Boolean) = ble.post {
    wantScan = uuids to lowPower
    startScanningNow(uuids, lowPower)
  }

  private fun startScanningNow(uuids: List<ByteArray>, lowPower: Boolean) {
    stopScanningNow()
    val sc = adapter?.bluetoothLeScanner ?: return
    val wanted = uuids.map { ParcelUuid(toUuid(it)) }.toSet()
    val filters = wanted.map { ScanFilter.Builder().setServiceUuid(it).build() }
    val settings = ScanSettings.Builder()
      .setScanMode(if (lowPower) ScanSettings.SCAN_MODE_LOW_POWER else ScanSettings.SCAN_MODE_BALANCED)
      .build()
    val cb = object : ScanCallback() {
      override fun onScanResult(callbackType: Int, result: ScanResult) {
        advertsSeen.incrementAndGet()
        val adv = result.scanRecord?.serviceUuids ?: return
        val matched = adv.filter { it in wanted }
        if (matched.isEmpty()) return
        val addr = result.device.address
        val h = scanHandles.computeIfAbsent(addr) { nextHandle.getAndIncrement() }
        ble.post {
          if (!links.containsKey(h)) links[h] = Link(h, result.device, true)
        }
        sink.onPeerFound(h, adv.map { toBytes(it.uuid) }, result.rssi)
      }
    }
    scanning = cb
    runCatching { sc.startScan(filters, settings, cb) }
  }

  fun stopScanning() = ble.post {
    wantScan = null
    stopScanningNow()
  }

  private fun stopScanningNow() {
    scanning?.let { cb -> runCatching { adapter?.bluetoothLeScanner?.stopScan(cb) } }
    scanning = null
  }

  // ===== Central role =====

  fun connect(handle: Long) = ble.post {
    val l = links[handle] ?: return@post
    if (l.gatt != null) return@post
    l.gatt = runCatching { l.device.connectGatt(context, false, centralCallback(l), BluetoothDevice.TRANSPORT_LE) }.getOrNull()
    if (l.gatt == null) sink.onDisconnected(handle)
  }

  private fun centralCallback(l: Link) = object : BluetoothGattCallback() {
    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
      ble.post {
        Diag.log("ble central: link ${l.handle} state=$newState status=$status")
        if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
          gatt.requestMtu(517)
        } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
          if (!l.up) failedConnects.incrementAndGet()
          closeLink(l, notify = true)
        }
      }
    }

    override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
      ble.post {
        l.mtu = mtu
        gatt.discoverServices()
      }
    }

    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
      ble.post {
        val svc = gatt.getService(DATA_SERVICE)
        val rx = svc?.getCharacteristic(RX)
        val tx = svc?.getCharacteristic(TX)
        if (rx == null || tx == null) {
          gatt.disconnect()
          return@post
        }
        l.rx = rx
        gatt.setCharacteristicNotification(tx, true)
        val d = tx.getDescriptor(CCCD)
        if (Build.VERSION.SDK_INT >= 33) {
          gatt.writeDescriptor(d, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
        } else {
          @Suppress("DEPRECATION")
          d.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
          @Suppress("DEPRECATION")
          gatt.writeDescriptor(d)
        }
      }
    }

    override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
      ble.post {
        if (status == BluetoothGatt.GATT_SUCCESS && !l.up) {
          l.up = true
          sink.onConnected(l.handle, l.mtu, true)
        }
      }
    }

    @Deprecated("API < 33")
    override fun onCharacteristicChanged(gatt: BluetoothGatt, c: BluetoothGattCharacteristic) {
      @Suppress("DEPRECATION")
      if (c.uuid == TX) sink.onFrame(l.handle, c.value.copyOf())
    }

    override fun onCharacteristicChanged(gatt: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
      if (c.uuid == TX) sink.onFrame(l.handle, value)
    }

    override fun onCharacteristicWrite(gatt: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
      ble.post {
        l.busy = false
        pump(l)
      }
    }
  }

  // ===== Peripheral role =====

  // The data service must really be registered: a process (re)started by the
  // system can open a GATT server that never gets its service, which leaves a
  // phone that advertises but cannot be connected to. Confirm it, and retry.
  private var serviceReady = false
  private var serverRetries = 0

  private fun ensureServer() {
    val cur = server
    if (cur != null && (serviceReady || cur.getService(DATA_SERVICE) != null)) return
    if (cur != null) {
      runCatching { cur.close() }
      server = null
      txChar = null
    }
    serviceReady = false
    val s = runCatching { manager.openGattServer(context, serverCallback) }.getOrNull() ?: return retryServer()
    val svc = BluetoothGattService(DATA_SERVICE, BluetoothGattService.SERVICE_TYPE_PRIMARY)
    val rx = BluetoothGattCharacteristic(RX, BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE or BluetoothGattCharacteristic.PROPERTY_WRITE, BluetoothGattCharacteristic.PERMISSION_WRITE)
    val tx = BluetoothGattCharacteristic(TX, BluetoothGattCharacteristic.PROPERTY_NOTIFY, BluetoothGattCharacteristic.PERMISSION_READ)
    tx.addDescriptor(BluetoothGattDescriptor(CCCD, BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE))
    svc.addCharacteristic(rx)
    svc.addCharacteristic(tx)
    txChar = tx
    server = s
    if (runCatching { s.addService(svc) }.getOrDefault(false) != true) return retryServer()
    // onServiceAdded normally confirms within milliseconds; check anyway.
    ble.postDelayed({
      if (server === s && !serviceReady && s.getService(DATA_SERVICE) == null) retryServer()
    }, 3_000)
  }

  private fun retryServer() {
    server?.let { runCatching { it.close() } }
    server = null
    txChar = null
    serviceReady = false
    val wait = (1_000L shl serverRetries.coerceAtMost(5)).coerceAtMost(30_000L)
    serverRetries++
    ble.postDelayed({ if (server == null) ensureServer() }, wait)
  }

  private val serverLinks = java.util.concurrent.ConcurrentHashMap<String, Long>() // address → handle for inbound links

  private val serverCallback = object : BluetoothGattServerCallback() {
    override fun onServiceAdded(status: Int, service: BluetoothGattService) {
      ble.post {
        if (service.uuid != DATA_SERVICE) return@post
        if (status == BluetoothGatt.GATT_SUCCESS) {
          serviceReady = true
          serverRetries = 0
        } else {
          retryServer()
        }
      }
    }

    override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
      ble.post {
        Diag.log("ble server: state=$newState status=$status known=${serverLinks.containsKey(device.address)}")
        if (newState == BluetoothProfile.STATE_CONNECTED) {
          val h = nextHandle.getAndIncrement()
          serverLinks[device.address] = h
          links[h] = Link(h, device, false)
          // Android keeps a Bluetooth connection only while some GATT
          // client holds it. When both phones connect at once and the
          // duplicate rule closes our client link, the shared connection
          // (and this inbound link with it) would drop. The server holds
          // its own reference; cancelConnection releases it.
          runCatching { server?.connect(device, false) }
        } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
          serverLinks.remove(device.address)?.let { h -> links[h]?.let { closeLink(it, notify = true) } }
        }
      }
    }

    override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
      ble.post { serverLinks[device.address]?.let { links[it]?.mtu = mtu } }
    }

    override fun onDescriptorWriteRequest(device: BluetoothDevice, requestId: Int, d: BluetoothGattDescriptor, prepared: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray) {
      if (responseNeeded) server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
      ble.post {
        if (d.uuid != CCCD) return@post
        val enable = value.contentEquals(BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
        var l = serverLinks[device.address]?.let { links[it] }
        if (!enable) {
          // The central unsubscribed: its session is over.
          if (l != null) {
            serverLinks.remove(device.address)
            closeLink(l, notify = true)
          }
          return@post
        }
        // A central that dropped and reconnected keeps Android's shared
        // Bluetooth connection, so no connect/disconnect event arrives here.
        // Subscribing again means a new session: replace any stale link.
        if (l == null || l.up) {
          if (l != null) {
            Diag.log("ble server: link ${l.handle} resubscribed, starting a new session")
            closeLink(l, notify = true)
          }
          val h = nextHandle.getAndIncrement()
          val fresh = Link(h, device, false)
          fresh.mtu = l?.mtu ?: fresh.mtu
          serverLinks[device.address] = h
          links[h] = fresh
          l = fresh
        }
        if (!l.up) {
          // Give the central up to 1 s to finish the ATT MTU exchange; a
          // 23-byte MTU cannot carry an encrypted link frame.
          fun announce(tries: Int) {
            if (l.up || !links.containsKey(l.handle)) return
            if (l.mtu >= 64 || tries == 0) {
              l.up = true
              Diag.log("ble server: link ${l.handle} up, mtu ${l.mtu}, ${l.early.size} early frames")
              sink.onConnected(l.handle, maxOf(l.mtu, 64), false)
              while (l.early.isNotEmpty()) sink.onFrame(l.handle, l.early.removeFirst())
            } else {
              ble.postDelayed({ announce(tries - 1) }, 100)
            }
          }
          announce(10)
        }
      }
    }

    override fun onCharacteristicWriteRequest(device: BluetoothDevice, requestId: Int, c: BluetoothGattCharacteristic, prepared: Boolean, responseNeeded: Boolean, offset: Int, value: ByteArray) {
      if (responseNeeded) server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, 0, null)
      if (c.uuid != RX) return
      ble.post {
        val h = serverLinks[device.address] ?: return@post
        val l = links[h] ?: return@post
        Diag.log("ble server: link $h rx ${value.size} B${if (l.up) "" else " (early)"}")
        if (l.up) sink.onFrame(h, value) else if (l.early.size < 64) l.early.addLast(value)
      }
    }

    override fun onNotificationSent(device: BluetoothDevice, status: Int) {
      ble.post {
        val l = serverLinks[device.address]?.let { links[it] } ?: return@post
        l.busy = false
        pump(l)
      }
    }
  }

  // ===== Sending =====

  fun sendFrame(handle: Long, bytes: ByteArray): Boolean {
    ble.post {
      val l = links[handle] ?: return@post
      if (l.queue.size > 512) {
        disconnectNow(l)
        return@post
      }
      l.queue.addLast(bytes)
      pump(l)
    }
    return true
  }

  private fun pump(l: Link) {
    if (l.busy || !l.up) return
    val next = l.queue.removeFirstOrNull() ?: return
    l.busy = true
    val ok = if (l.central) {
      val g = l.gatt
      val rx = l.rx
      if (g == null || rx == null) false
      else if (Build.VERSION.SDK_INT >= 33) {
        g.writeCharacteristic(rx, next, BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE) == BluetoothStatusCodes.SUCCESS
      } else {
        @Suppress("DEPRECATION")
        rx.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        @Suppress("DEPRECATION")
        rx.value = next
        @Suppress("DEPRECATION")
        g.writeCharacteristic(rx)
      }
    } else {
      val s = server
      val tx = txChar
      if (s == null || tx == null) false
      else if (Build.VERSION.SDK_INT >= 33) {
        s.notifyCharacteristicChanged(l.device, tx, false, next) == BluetoothStatusCodes.SUCCESS
      } else {
        @Suppress("DEPRECATION")
        tx.value = next
        @Suppress("DEPRECATION")
        s.notifyCharacteristicChanged(l.device, tx, false)
      }
    }
    if (!ok) {
      Diag.log("ble: link ${l.handle} send busy, retrying")
      // Retry shortly; the stack is momentarily busy.
      l.queue.addFirst(next)
      l.busy = false
      ble.postDelayed({ pump(l) }, 15)
    }
  }

  fun disconnect(handle: Long) = ble.post { links[handle]?.let { disconnectNow(it) } }

  private fun disconnectNow(l: Link) {
    Diag.log("ble: link ${l.handle} closed by us (${if (l.central) "central" else "server"})")
    if (l.central) l.gatt?.disconnect() else server?.cancelConnection(l.device)
    closeLink(l, notify = false)
  }

  private fun closeLink(l: Link, notify: Boolean) {
    val wasUp = l.up
    Diag.log("ble: link ${l.handle} closed, was up $wasUp, ${l.queue.size} queued")
    l.up = false
    l.queue.clear()
    l.busy = false
    if (l.central) {
      l.gatt?.close()
      l.gatt = null
    }
    if (!l.central) links.remove(l.handle)
    if (notify || wasUp) sink.onDisconnected(l.handle)
  }

  fun shutdown() = ble.post {
    runCatching { context.unregisterReceiver(radioState) }
    wantAdvert = null
    wantScan = null
    stopAdvertisingNow()
    stopScanningNow()
    for (l in links.values.toList()) disconnectNow(l)
    server?.close()
    server = null
  }
}
