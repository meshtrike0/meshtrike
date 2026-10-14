package expo.modules.meshcore

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

/**
 * Foreground service of type connectedDevice that keeps BLE relaying alive
 * (section 9, required on Android 14+). The notification never shows
 * message content or names (section 17).
 */
class RelayService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
    if (Build.VERSION.SDK_INT >= 26) {
      nm.createNotificationChannel(NotificationChannel(CHANNEL, "Relay", NotificationManager.IMPORTANCE_MIN))
    }
    val n: Notification = NotificationCompat.Builder(this, CHANNEL)
      .setContentTitle("Relaying nearby messages")
      .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
      .setOngoing(true)
      .setVisibility(NotificationCompat.VISIBILITY_SECRET)
      .build()
    // During a call the microphone stays on if the screen goes off.
    val mic = intent?.getBooleanExtra(EXTRA_CALL, false) ?: false
    runCatching {
      if (Build.VERSION.SDK_INT >= 30 && mic) {
        startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
      } else if (Build.VERSION.SDK_INT >= 29) {
        startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
      } else {
        startForeground(1, n)
      }
    }
    // Relay works after first unlock even if the UI never opens (section 10).
    runCatching { MeshRuntime.start(applicationContext) }
    return START_STICKY
  }

  companion object {
    const val CHANNEL = "mesh-relay"
    private const val EXTRA_CALL = "call"

    /** Re-declare the service with (or without) the microphone for a call. */
    fun setCallActive(ctx: Context, on: Boolean) {
      runCatching {
        androidx.core.content.ContextCompat.startForegroundService(ctx, Intent(ctx, RelayService::class.java).putExtra(EXTRA_CALL, on))
      }
    }
  }
}

/** Restart relaying after reboot (BOOT_COMPLETED fires after first unlock). */
class BootReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    if (intent.action == Intent.ACTION_BOOT_COMPLETED) {
      runCatching {
        androidx.core.content.ContextCompat.startForegroundService(context, Intent(context, RelayService::class.java))
      }
    }
  }
}
