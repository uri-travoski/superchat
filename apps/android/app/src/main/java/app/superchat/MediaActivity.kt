package app.superchat

import android.Manifest
import android.app.AlertDialog
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Typeface
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Bundle
import android.provider.OpenableColumns
import android.util.Base64
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.video.FileOutputOptions
import androidx.camera.video.PendingRecording
import androidx.camera.video.Quality
import androidx.camera.video.QualitySelector
import androidx.camera.video.Recorder
import androidx.camera.video.Recording
import androidx.camera.video.VideoCapture
import androidx.camera.video.VideoRecordEvent
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID

/**
 * M-31: everything that puts files into a message on Android.
 *  - mode "files": the system file picker (any type, several at once)
 *  - mode "gallery": the system photo picker (pictures and videos)
 *  - mode "image": the system photo picker for one picture (profile photo)
 *  - mode "video": an in-app video recorder (CameraX). The server's upload limit is shown before recording starts, the
 *    size grows on screen while recording, and the clip stops by itself just under the limit.
 * Picked files are copied into the app's cache; for videos the duration, size and a poster frame are computed here and
 * handed to the Rust app with the path, which uploads them and sends the message.
 */
class MediaActivity : ComponentActivity() {
    private var limit = 100L * 1024 * 1024
    private var preview: PreviewView? = null
    private var provider: ProcessCameraProvider? = null
    private var videoCapture: VideoCapture<Recorder>? = null
    private var recording: Recording? = null
    private var front = false
    private var recordFile: File? = null
    private var stoppedBySize = false
    private lateinit var status: TextView
    private lateinit var bar: ProgressBar
    private lateinit var recordButton: Button
    private lateinit var flipButton: Button

    private val pickFiles = registerForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris -> handlePicked(uris) }
    private val pickOne = registerForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri -> handlePicked(listOfNotNull(uri)) }
    private val pickGallery = registerForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(10)) { uris -> handlePicked(uris) }
    private val askCamera = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        if (granted[Manifest.permission.CAMERA] == true) startCamera()
        else { Toast.makeText(this, "The camera permission was refused.", Toast.LENGTH_LONG).show(); finish() }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        limit = intent.getLongExtra("limit", limit)
        cleanOld()
        when (intent.getStringExtra("mode")) {
            "files" -> { setContentView(FrameLayout(this)); pickFiles.launch("*/*") }
            "image" -> { setContentView(FrameLayout(this)); pickOne.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) }
            "gallery" -> { setContentView(FrameLayout(this)); pickGallery.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) }
            "video" -> {
                buildRecorderUi()
                if (!packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) {
                    Toast.makeText(this, "This device has no camera.", Toast.LENGTH_LONG).show(); finish(); return
                }
                val have = checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED &&
                    checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
                if (have) startCamera() else rationale()
            }
            else -> finish()
        }
    }

    private fun mb(bytes: Long) = String.format("%.1f MB", bytes / 1048576.0)

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

    // ---- recorder ---------------------------------------------------------------------------------------------------

    private fun buildRecorderUi() {
        val root = FrameLayout(this)
        root.setBackgroundColor(Color.BLACK)
        val pv = PreviewView(this)
        preview = pv
        root.addView(pv, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        val top = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setBackgroundColor(0x99000000.toInt()); setPadding(32, 120, 32, 24) }
        status = TextView(this).apply {
            text = "Video clip. Up to ${mb(limit)} can be sent; recording stops by itself at that size."
            setTextColor(Color.WHITE); textSize = 15f; gravity = Gravity.CENTER
        }
        bar = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply { max = 1000; progress = 0 }
        top.addView(status); top.addView(bar)
        root.addView(top, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP))
        val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER; setPadding(0, 0, 0, 24) }
        val cancel = Button(this).apply { text = "Cancel"; contentDescription = "Cancel and discard"; setOnClickListener { discardAndFinish() } }
        recordButton = Button(this).apply {
            text = "Record"; contentDescription = "Start recording"; setTypeface(typeface, Typeface.BOLD)
            setTextColor(Color.WHITE); setBackgroundColor(0xFFD32F2F.toInt()); setPadding(64, 24, 64, 24)
            setOnClickListener { toggleRecording() }
        }
        flipButton = Button(this).apply { text = "Flip"; contentDescription = "Switch between the front and back camera"; setOnClickListener { front = !front; bindCamera() } }
        row.addView(cancel); row.addView(recordButton); row.addView(flipButton)
        val lp = FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM)
        lp.bottomMargin = 96
        root.addView(row, lp)
        setContentView(root)
    }

    private fun rationale() {
        AlertDialog.Builder(this)
            .setTitle("Allow camera and microphone?")
            .setMessage("SuperChat records the clip on this phone and sends it only to the conversation you choose. The camera and microphone are used only while you are on this screen.")
            .setPositiveButton("Continue") { _, _ -> askCamera.launch(arrayOf(Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO)) }
            .setNegativeButton("Not now") { _, _ -> finish() }
            .setOnCancelListener { finish() }
            .show()
    }

    private fun startCamera() {
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            try { provider = future.get(); bindCamera() }
            catch (e: Exception) { Toast.makeText(this, "The camera could not start: ${e.message}", Toast.LENGTH_LONG).show(); finish() }
        }, ContextCompat.getMainExecutor(this))
    }

    private fun bindCamera() {
        val p = provider ?: return
        val pv = preview ?: return
        if (recording != null) return
        val recorder = Recorder.Builder()
            // 480p keeps clips small enough to send over mobile data; fall back to whatever the camera offers.
            .setQualitySelector(QualitySelector.fromOrderedList(listOf(Quality.SD, Quality.HD, Quality.LOWEST)))
            .build()
        val vc = VideoCapture.withOutput(recorder)
        videoCapture = vc
        val prev = Preview.Builder().build().also { it.setSurfaceProvider(pv.surfaceProvider) }
        p.unbindAll()
        val sel = if (front) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA
        try { p.bindToLifecycle(this, sel, prev, vc) }
        catch (e: Exception) {
            if (front) { front = false; bindCamera() }
            else { Toast.makeText(this, "No camera is available.", Toast.LENGTH_LONG).show(); finish() }
        }
    }

    private fun toggleRecording() {
        val active = recording
        if (active != null) { active.stop(); recordButton.isEnabled = false; return }
        val vc = videoCapture ?: return
        val dir = File(cacheDir, "outgoing/" + UUID.randomUUID()).also { it.mkdirs() }
        val f = File(dir, "video-" + java.text.SimpleDateFormat("yyyyMMdd-HHmmss", java.util.Locale.US).format(java.util.Date()) + ".mp4")
        recordFile = f
        stoppedBySize = false
        // A margin below the limit leaves room for the index the MP4 writer adds at the end.
        val opts = FileOutputOptions.Builder(f).setFileSizeLimit((limit - limit / 20).coerceAtLeast(1L)).build()
        var pr: PendingRecording = vc.output.prepareRecording(this, opts)
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) pr = pr.withAudioEnabled()
        recording = pr.start(ContextCompat.getMainExecutor(this)) { ev ->
            when (ev) {
                is VideoRecordEvent.Start -> {
                    recordButton.text = "Stop"; recordButton.contentDescription = "Stop recording and send"; flipButton.isEnabled = false
                }
                is VideoRecordEvent.Status -> {
                    val bytes = ev.recordingStats.numBytesRecorded
                    val secs = ev.recordingStats.recordedDurationNanos / 1_000_000_000L
                    status.text = String.format("Recording %d:%02d   %s of %s", secs / 60, secs % 60, mb(bytes), mb(limit))
                    bar.progress = (bytes * 1000 / limit.coerceAtLeast(1)).toInt().coerceIn(0, 1000)
                }
                is VideoRecordEvent.Finalize -> {
                    recording = null
                    stoppedBySize = ev.error == VideoRecordEvent.Finalize.ERROR_FILE_SIZE_LIMIT_REACHED
                    val ok = !ev.hasError() || stoppedBySize
                    if (ok && f.exists() && f.length() > 0) finishRecording(f)
                    else { Toast.makeText(this, "Nothing was recorded.", Toast.LENGTH_LONG).show(); f.delete(); finish() }
                }
                else -> {}
            }
        }
    }

    private fun finishRecording(f: File) {
        status.text = "Preparing the clip..."
        Thread {
            val meta = videoMeta(f)?.toString() ?: ""
            runOnUiThread {
                if (stoppedBySize) Toast.makeText(this, "Reached the size limit of ${mb(limit)}; the clip was cut there.", Toast.LENGTH_LONG).show()
                Native.nativePicked(f.absolutePath, meta)
                finish()
            }
        }.start()
    }

    private fun discardAndFinish() {
        try { recording?.close() } catch (_: Exception) {}
        recording = null
        recordFile?.delete()
        finish()
    }

    override fun onDestroy() {
        super.onDestroy()
        try { recording?.close() } catch (_: Exception) {}
    }
}
