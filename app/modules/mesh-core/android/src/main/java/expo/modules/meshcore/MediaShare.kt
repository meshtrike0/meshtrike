package expo.modules.meshcore

import android.app.Activity
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.media.MediaDataSource
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.util.Base64
import androidx.core.content.FileProvider
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.nio.ByteBuffer

/**
 * Files and photos leaving or entering the app at the user's request.
 *
 * - A picked file is read and handed to the core here, off the JS thread, so
 *   a large video does not freeze the screen while it is sealed into parts.
 * - A received photo is only ever exported as a fresh JPEG made from the
 *   core's decoded pixels: never the received bytes, so nothing hidden in the
 *   original (metadata, a malformed header) leaves with it.
 */
internal object MediaShare {
  /** Same limit as the core (MAX_FILE_BYTES); checked before reading it all. */
  private const val MAX_FILE_BYTES = 5 * 1024 * 1024

  fun sendFile(ctx: Context, conversation: String, uri: String, name: String, optionsJson: String): String {
    val bytes = ctx.contentResolver.openInputStream(Uri.parse(uri))?.use { input ->
      val out = ByteArrayOutputStream()
      val buf = ByteArray(64 * 1024)
      while (true) {
        val n = input.read(buf)
        if (n < 0) break
        out.write(buf, 0, n)
        if (out.size() > MAX_FILE_BYTES) throw IllegalArgumentException("Files can be up to 5 MB")
      }
      out.toByteArray()
    } ?: throw IllegalArgumentException("Could not read that file")
    // Gallery picks often come named "video1000026508": add the extension of
    // its type so the other phone knows what it is (a video, a PDF…).
    val ext = android.webkit.MimeTypeMap.getSingleton().getExtensionFromMimeType(ctx.contentResolver.getType(Uri.parse(uri)) ?: "")
    val named = if (!name.contains('.') && !ext.isNullOrEmpty()) "$name.$ext" else name
    val args = JSONObject()
      .put("conversation", conversation)
      .put("name", named)
      .put("bytes", Base64.encodeToString(bytes, Base64.NO_WRAP))
      .put("options", JSONObject(optionsJson))
    return MeshRuntime.call("send_file", args.toString())
  }

  /** A received or sent photo as a new JPEG, from the core's pixels. */
  private fun jpegOf(message: String): ByteArray {
    val reply = JSONObject(MeshRuntime.call("image_pixels", JSONObject().put("message", message).toString()))
    val img = reply.optJSONObject("ok") ?: throw IllegalStateException("Photo not available")
    val w = img.getInt("width")
    val h = img.getInt("height")
    val rgba = Base64.decode(img.getString("rgba"), Base64.DEFAULT)
    if (w <= 0 || h <= 0 || rgba.size != w * h * 4) throw IllegalStateException("Photo not available")
    // ARGB_8888 keeps its pixels as R, G, B, A bytes in memory.
    val bmp = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
    bmp.copyPixelsFromBuffer(ByteBuffer.wrap(rgba))
    val out = ByteArrayOutputStream()
    bmp.compress(Bitmap.CompressFormat.JPEG, 92, out)
    bmp.recycle()
    return out.toByteArray()
  }

  /** Into the phone's Photos, in a "Meshtrike" album (no permission needed). */
  fun savePhoto(ctx: Context, message: String): Boolean {
    val jpeg = jpegOf(message)
    val values = ContentValues().apply {
      put(MediaStore.Images.Media.DISPLAY_NAME, "meshtrike-${System.currentTimeMillis()}.jpg")
      put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg")
      put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/Meshtrike")
      put(MediaStore.Images.Media.IS_PENDING, 1)
    }
    val r = ctx.contentResolver
    val uri = r.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values) ?: throw IllegalStateException("Could not save")
    try {
      r.openOutputStream(uri)?.use { it.write(jpeg) } ?: throw IllegalStateException("Could not save")
      values.clear()
      values.put(MediaStore.Images.Media.IS_PENDING, 0)
      r.update(uri, values, null, null)
    } catch (e: Exception) {
      r.delete(uri, null, null)
      throw e
    }
    return true
  }

  /** Android's share sheet, with a short-lived copy the receiving app may read. */
  fun sharePhoto(ctx: Context, activity: Activity?, message: String): Boolean {
    val dir = File(ctx.cacheDir, "shared").apply { mkdirs() }
    // Earlier shares are removed; the receiving app has had its chance to read them.
    dir.listFiles()?.forEach { if (System.currentTimeMillis() - it.lastModified() > 10 * 60_000) it.delete() }
    val f = File(dir, "photo-${System.currentTimeMillis()}.jpg")
    f.writeBytes(jpegOf(message))
    val uri = FileProvider.getUriForFile(ctx, ctx.packageName + ".meshshare", f)
    val send = Intent(Intent.ACTION_SEND).apply {
      type = "image/jpeg"
      putExtra(Intent.EXTRA_STREAM, uri)
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    val chooser = Intent.createChooser(send, null).apply { addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION) }
    if (activity != null) activity.startActivity(chooser) else ctx.startActivity(chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    return true
  }

  private val VIDEO_TYPES = mapOf(
    "mp4" to "video/mp4", "m4v" to "video/mp4", "mov" to "video/quicktime",
    "3gp" to "video/3gpp", "webm" to "video/webm", "mkv" to "video/x-matroska",
  )

  private fun fileOf(message: String): Pair<String, ByteArray> {
    val reply = JSONObject(MeshRuntime.call("file_bytes", JSONObject().put("message", message).toString()))
    val ok = reply.optJSONObject("ok") ?: throw IllegalStateException("File not available")
    return ok.getString("name") to Base64.decode(ok.getString("bytes"), Base64.DEFAULT)
  }

  private fun videoType(name: String): String =
    VIDEO_TYPES[name.substringAfterLast('.', "").lowercase()] ?: throw IllegalArgumentException("Not a video")

  /**
   * A received video's length and size, for its bubble. Android's media
   * service reads the header in its own process; nothing is played here.
   */
  fun videoInfo(message: String): Map<String, Any> {
    val (name, bytes) = fileOf(message)
    videoType(name)
    val r = MediaMetadataRetriever()
    try {
      r.setDataSource(object : MediaDataSource() {
        override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
          if (position >= bytes.size) return -1
          val n = minOf(size.toLong(), bytes.size - position).toInt()
          System.arraycopy(bytes, position.toInt(), buffer, offset, n)
          return n
        }
        override fun getSize(): Long = bytes.size.toLong()
        override fun close() {}
      })
      val num = { k: Int -> r.extractMetadata(k)?.toLongOrNull() ?: 0L }
      return mapOf(
        "durationMs" to num(MediaMetadataRetriever.METADATA_KEY_DURATION),
        "width" to num(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH),
        "height" to num(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT),
      )
    } catch (e: Exception) {
      Diag.log("video: length unreadable (${e.javaClass.simpleName})")
      return mapOf("durationMs" to 0L, "width" to 0L, "height" to 0L)
    } finally {
      runCatching { r.release() }
    }
  }

  /**
   * Meshtrike never plays videos itself: the user picks a player, which gets
   * a short-lived copy it may read (removed after 10 minutes or at the next
   * start).
   */
  fun openVideo(ctx: Context, activity: Activity?, message: String): Boolean {
    val (name, bytes) = fileOf(message)
    val type = videoType(name)
    val dir = File(ctx.cacheDir, "shared").apply { mkdirs() }
    dir.listFiles()?.forEach { if (System.currentTimeMillis() - it.lastModified() > 10 * 60_000) it.delete() }
    val f = File(dir, "video-${System.currentTimeMillis()}.${name.substringAfterLast('.').lowercase()}")
    f.writeBytes(bytes)
    val uri = FileProvider.getUriForFile(ctx, ctx.packageName + ".meshshare", f)
    val view = Intent(Intent.ACTION_VIEW).apply {
      setDataAndType(uri, type)
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    val chooser = Intent.createChooser(view, null).apply { addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION) }
    if (activity != null) activity.startActivity(chooser) else ctx.startActivity(chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    return true
  }

  /** Shared copies never outlive the app's next start. */
  fun clearShared(ctx: Context) {
    File(ctx.cacheDir, "shared").listFiles()?.forEach { it.delete() }
  }
}
