package app.superchat

import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.util.Base64
import android.widget.FrameLayout
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID

/**
 * M-31: everything that puts files into a message on Android.
 *  - mode "files": the system file picker (any type, several at once)
 *  - mode "gallery": the system photo picker (pictures and videos)
 *  - mode "image": the system photo picker for one picture (profile photo)
 * (1.4: the in-app video recorder was removed. Videos are still sent from the photo picker or the file picker.)
 * Picked files are copied into the app's cache; for videos the duration, size and a poster frame are computed here and
 * handed to the Rust app with the path, which uploads them and sends the message.
 */
class MediaActivity : ComponentActivity() {
    private var limit = 100L * 1024 * 1024

    private val pickFiles = registerForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris -> handlePicked(uris) }
    private val pickOne = registerForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri -> handlePicked(listOfNotNull(uri)) }
    private val pickGallery = registerForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(10)) { uris -> handlePicked(uris) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        limit = intent.getLongExtra("limit", limit)
        cleanOld()
        when (intent.getStringExtra("mode")) {
            "files" -> { setContentView(FrameLayout(this)); pickFiles.launch("*/*") }
            "image" -> { setContentView(FrameLayout(this)); pickOne.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }
            "gallery" -> { setContentView(FrameLayout(this)); pickGallery.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) }
            else -> finish()
        }
    }

    // ---- picker results ---------------------------------------------------------------------------------------------

    private fun handlePicked(uris: List<Uri>) {
        if (uris.isEmpty()) { finish(); return }
        Thread {
            val paths = ArrayList<String>()
            val metas = ArrayList<String>()
            for (u in uris) {
                val f = copyToCache(u) ?: continue
                paths.add(f.absolutePath)
                metas.add(if ((contentResolver.getType(u) ?: "").startsWith("video/")) videoMeta(f)?.toString() ?: "" else "")
            }
            runOnUiThread {
                if (paths.isNotEmpty()) Native.nativePicked(paths.joinToString("\n"), metas.joinToString("\n"))
                else Toast.makeText(this, "Those files could not be read.", Toast.LENGTH_LONG).show()
                finish()
            }
        }.start()
    }

    private fun copyToCache(u: Uri): File? = try {
        var name = "file"
        contentResolver.query(u, null, null, null, null)?.use { c ->
            val i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (i >= 0 && c.moveToFirst()) name = c.getString(i) ?: name
        }
        name = name.replace('/', '_')
        val dir = File(cacheDir, "outgoing/" + UUID.randomUUID()).also { it.mkdirs() }
        val out = File(dir, name)
        contentResolver.openInputStream(u)?.use { input -> out.outputStream().use { input.copyTo(it) } }
        out
    } catch (e: Exception) { android.util.Log.e("atomic", "copy failed", e); null }

    /** Clips and copies from earlier sends are no longer needed after a day. */
    private fun cleanOld() {
        try {
            val root = File(cacheDir, "outgoing")
            val cutoff = System.currentTimeMillis() - 24L * 3600 * 1000
            root.listFiles()?.filter { it.lastModified() < cutoff }?.forEach { it.deleteRecursively() }
        } catch (_: Exception) {}
    }

    // ---- video details ----------------------------------------------------------------------------------------------

    /** Duration, size and a poster frame (a small JPEG, base64) as the JSON the server stores with the attachment. */
    private fun videoMeta(f: File): JSONObject? {
        val r = MediaMetadataRetriever()
        return try {
            r.setDataSource(f.absolutePath)
            val dur = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() ?: 0L
            var w = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull() ?: 0
            var h = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull() ?: 0
            val rot = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
            if (rot == 90 || rot == 270) { val t = w; w = h; h = t }
            val j = JSONObject()
            if (dur > 0) j.put("duration_ms", dur)
            if (w > 0 && h > 0) { j.put("width", w); j.put("height", h) }
            val frame = r.getFrameAtTime(if (dur > 3000) 1_000_000L else 0L, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)
            if (frame != null) {
                val tw = 320
                val th = (frame.height * tw / frame.width.coerceAtLeast(1)).coerceAtLeast(1)
                val small = Bitmap.createScaledBitmap(frame, tw, th, true)
                var q = 70
                var bytes: ByteArray
                do {
                    val bo = ByteArrayOutputStream()
                    small.compress(Bitmap.CompressFormat.JPEG, q, bo)
                    bytes = bo.toByteArray()
                    q -= 15
                } while (bytes.size > 40 * 1024 && q > 10)
                if (bytes.size <= 40 * 1024) j.put("poster_b64", Base64.encodeToString(bytes, Base64.NO_WRAP))
            }
            j
        } catch (e: Exception) { android.util.Log.e("atomic", "video details failed", e); null } finally { try { r.release() } catch (_: Exception) {} }
    }
}
