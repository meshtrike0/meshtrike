package expo.modules.meshcore

import android.annotation.SuppressLint
import android.bluetooth.*
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelUuid
import java.io.File
import java.util.UUID

/**
 * Bluetooth link to an off-the-shelf Meshtastic radio (section 9). The radio
 * is an untrusted pipe: we only hand it sealed 233-byte envelopes wrapped by
 * the core (lora_encode) and pass PRIVATE_APP payloads back (lora_decode).
 * Region, ISM band and transmit power are configured on the radio itself.
 */
@SuppressLint("MissingPermission")
class LoraBridge(private val context: Context, private val onEnvelope: (ByteArray) -> Unit, private val encode: (ByteArray) -> ByteArray?, private val decode: (ByteArray) -> ByteArray?) {
  companion object {
    val SERVICE: UUID = UUID.fromString("6ba1b218-15a8-461f-9fa8-5dcae273eafd")
    val TO_RADIO: UUID = UUID.fromString("f75c76d2-129e-4dad-a1dd-7866124401e7")
    val FROM_RADIO: UUID = UUID.fromString("2c55e69e-4993-11ed-b878-0242ac120002")
    val FROM_NUM: UUID = UUID.fromString("ed9da18c-a800-4f66-a670-aa7547e34453")
    val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
  }

  private val thread = HandlerThread("mesh-lora").apply { start() }
  private val h = Handler(thread.looper)
  private val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager
  private val saved = File(context.noBackupFilesDir, "lora_radio")
  private var gatt: BluetoothGatt? = null
  private var toRadio: BluetoothGattCharacteristic? = null
  private var fromRadio: BluetoothGattCharacteristic? = null
  private val outQueue = ArrayDeque<ByteArray>()
  private var busy = false
  @Volatile var connected = false
    private set
  @Volatile var radioName: String? = null
    private set

  fun restore() = h.post { saved.takeIf { it.exists() }?.readText()?.trim()?.takeIf { it.isNotEmpty() }?.let { connectNow(it) } }

  fun scan(onResult: (List<Pair<String, String>>) -> Unit) = h.post {
    val sc = manager.adapter?.bluetoothLeScanner ?: return@post onResult(emptyList())
    val found = LinkedHashMap<String, String>()
    val cb = object : ScanCallback() {
      override fun onScanResult(t: Int, r: ScanResult) {
        found[r.device.address] = r.device.name ?: r.scanRecord?.deviceName ?: "Meshtastic radio"
      }
    }
    sc.startScan(listOf(ScanFilter.Builder().setServiceUuid(ParcelUuid(SERVICE)).build()), ScanSettings.Builder().build(), cb)
    h.postDelayed({
      runCatching { sc.stopScan(cb) }
      onResult(found.map { it.key to it.value })
    }, 8000)
  }

  fun connect(address: String) = h.post {
    saved.writeText(address)
    connectNow(address)
  }

  fun forget() = h.post {
    saved.delete()
    gatt?.disconnect()
    gatt?.close()
    gatt = null
    connected = false
    radioName = null
  }

  private fun connectNow(address: String) {
    val dev = runCatching { manager.adapter?.getRemoteDevice(address) }.getOrNull() ?: return
    gatt?.close()
    gatt = dev.connectGatt(context, true, callback, BluetoothDevice.TRANSPORT_LE)
  }

  fun send(envelope: ByteArray): Boolean {
    if (!connected) return false
    val msg = encode(envelope) ?: return false
    h.post {
      outQueue.addLast(msg)
      pump()
    }
    return true
  }

  private fun pump() {
    if (busy) return
    val g = gatt ?: return
    val c = toRadio ?: return
    val next = outQueue.removeFirstOrNull() ?: return
    busy = true
    val ok = if (Build.VERSION.SDK_INT >= 33) {
      g.writeCharacteristic(c, next, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT) == BluetoothStatusCodes.SUCCESS
    } else {
      @Suppress("DEPRECATION")
      c.value = next
      @Suppress("DEPRECATION")
      g.writeCharacteristic(c)
    }
    if (!ok) {
      busy = false
      outQueue.addFirst(next)
      h.postDelayed({ pump() }, 100)
    }
  }

  private fun drainFromRadio() {
    val g = gatt ?: return
    val c = fromRadio ?: return
    if (!busy) {
      busy = true
      if (!g.readCharacteristic(c)) busy = false
    }
  }

  private val callback = object : BluetoothGattCallback() {
    override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
      h.post {
        if (newState == BluetoothProfile.STATE_CONNECTED) {
          radioName = g.device.name
          g.requestMtu(512)
        } else {
          connected = false
          busy = false
        }
      }
    }

    override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
      h.post { g.discoverServices() }
    }

    override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
      h.post {
        val s = g.getService(SERVICE) ?: return@post
        toRadio = s.getCharacteristic(TO_RADIO)
        fromRadio = s.getCharacteristic(FROM_RADIO)
        val num = s.getCharacteristic(FROM_NUM) ?: return@post
        g.setCharacteristicNotification(num, true)
        val d = num.getDescriptor(CCCD) ?: return@post
        if (Build.VERSION.SDK_INT >= 33) g.writeDescriptor(d, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE)
        else {
          @Suppress("DEPRECATION")
          d.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
          @Suppress("DEPRECATION")
          g.writeDescriptor(d)
        }
      }
    }

    override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
      h.post {
        connected = status == BluetoothGatt.GATT_SUCCESS
        drainFromRadio()
      }
    }

    @Deprecated("API < 33")
    override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
      if (c.uuid == FROM_NUM) h.post { drainFromRadio() }
    }

    override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
      if (c.uuid == FROM_NUM) h.post { drainFromRadio() }
    }

    @Deprecated("API < 33")
    override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
      @Suppress("DEPRECATION")
      handleRead(c.value ?: ByteArray(0))
    }

    override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) {
      handleRead(value)
    }

    override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
      h.post {
        busy = false
        pump()
      }
    }
  }

  private fun handleRead(value: ByteArray) = h.post {
    busy = false
    if (value.isNotEmpty()) {
      decode(value)?.let(onEnvelope)
      drainFromRadio() // keep reading until the radio's queue is empty
    } else {
      pump()
    }
  }
}
