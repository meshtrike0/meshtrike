package expo.modules.meshcore

import android.content.Context
import android.util.Base64
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.util.concurrent.Executors

/**
 * Camera preview whose frames (Y plane = 8-bit luma) are decoded by the Rust
 * core (D25). No ML Kit, no Google Play services, no network.
 */
class QrScannerView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  // Children are native views: let Android measure and lay them out.
  override val shouldUseAndroidLayout = true
  private val onCode by EventDispatcher()
  private val preview = PreviewView(context).apply {
    layoutParams = LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)
    // TextureView embeds reliably inside the React Native view hierarchy.
    implementationMode = PreviewView.ImplementationMode.COMPATIBLE
    scaleType = PreviewView.ScaleType.FILL_CENTER
  }
  private val executor = Executors.newSingleThreadExecutor()
  private var provider: ProcessCameraProvider? = null
  private var intervalMs = 125L
  private var last = 0L

  init {
    addView(preview)
  }

  fun setFps(fps: Int) {
    intervalMs = (1000L / fps.coerceIn(1, 15))
  }

  override fun onAttachedToWindow() {
    super.onAttachedToWindow()
    val owner = appContext.currentActivity as? LifecycleOwner ?: return
    val future = ProcessCameraProvider.getInstance(context)
    future.addListener({
      val p = future.get()
      provider = p
      val pv = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
      val analysis = ImageAnalysis.Builder()
        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
        .build()
      analysis.setAnalyzer(executor) { img ->
        try {
          val now = System.currentTimeMillis()
          if (now - last < intervalMs) return@setAnalyzer
          last = now
          val plane = img.planes[0]
          val w = img.width
          val h = img.height
          val row = plane.rowStride
          val buf = plane.buffer
          val luma = ByteArray(w * h)
          for (y in 0 until h) {
            buf.position(y * row)
            buf.get(luma, y * w, w)
          }
          for (code in MeshRuntime.decodeQr(w, h, luma)) {
            post { onCode(mapOf("base64" to Base64.encodeToString(code, Base64.NO_WRAP))) }
          }
        } catch (_: Throwable) {
        } finally {
          img.close()
        }
      }
      runCatching {
        p.unbindAll()
        p.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, pv, analysis)
      }
    }, ContextCompat.getMainExecutor(context))
  }

  override fun onDetachedFromWindow() {
    super.onDetachedFromWindow()
    runCatching { provider?.unbindAll() }
  }
}

class MeshQrScannerModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("MeshQrScanner")
    View(QrScannerView::class) {
      Events("onCode")
      Prop("fps") { view: QrScannerView, fps: Int -> view.setFps(fps) }
    }
  }
}
