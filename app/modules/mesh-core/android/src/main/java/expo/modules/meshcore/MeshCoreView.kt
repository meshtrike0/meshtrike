package expo.modules.meshcore

import android.content.Context
import android.graphics.Bitmap
import android.util.Base64
import android.widget.ImageView
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.views.ExpoView
import java.nio.ByteBuffer

/**
 * Shows raw RGBA pixels decoded by the Rust core (D31). The bytes go straight
 * into a Bitmap with copyPixelsFromBuffer: no image decoder runs.
 */
class MeshCoreView(context: Context, appContext: AppContext) : ExpoView(context, appContext) {
  override val shouldUseAndroidLayout = true
  private val image = ImageView(context).apply {
    layoutParams = LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)
    scaleType = ImageView.ScaleType.FIT_CENTER
  }
  private var rgba: String? = null
  private var w = 0
  private var h = 0

  init {
    addView(image)
  }

  fun setRgba(v: String) {
    rgba = v
    render()
  }

  fun setPixelWidth(v: Int) {
    w = v
    render()
  }

  fun setPixelHeight(v: Int) {
    h = v
    render()
  }

  private fun render() {
    val data = rgba ?: return
    if (w <= 0 || h <= 0 || w > 1600 || h > 1600) return
    val px = Base64.decode(data, Base64.NO_WRAP)
    if (px.size != w * h * 4) return
    val bmp = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
    bmp.copyPixelsFromBuffer(ByteBuffer.wrap(px))
    image.setImageBitmap(bmp)
  }
}
