package app.superchat

import android.Manifest
import android.app.AlertDialog
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.view.ViewGroup
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.InvertedLuminanceSource
import com.google.zxing.MultiFormatReader
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.Executors

/**
 * Reads the QR code of a one-time login link (admin panel, Users, Create user / Login link): live with CameraX, or from a
 * picture chosen in the photo picker. Decoding is done in memory with ZXing; nothing leaves the device. The text goes to
 * the Rust app, which signs in with it. Every failure is reported to the login screen as text.
 */
class ScanActivity : ComponentActivity() {
    private val analysisExecutor = Executors.newSingleThreadExecutor()
    private val hints = mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true)
    private val reader = MultiFormatReader().apply { setHints(hints) }
    @Volatile private var done = false
    private var preview: PreviewView? = null

    private val pickImage = registerForActivityResult(ActivityResultContracts.GetContent()) { uri: Uri? ->
        if (uri == null) { if (intent.getBooleanExtra("image", false)) finish() } else decodePicture(uri)
    }
    private val askCamera = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) startCamera() else fail("The camera permission was refused. Paste the link, or choose a picture of the QR code.")
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        title = "Scan QR code"
        if (intent.getBooleanExtra("image", false)) {
            // Only the picker; the (empty) window stays behind it.
            setContentView(FrameLayout(this))
            pickImage.launch("image/*")
            return
        }
        buildCameraUi()
        if (!packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) {
            fail("This device has no camera. Paste the link, or choose a picture of the QR code.")
            return
        }
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) startCamera()
        else rationale()
    }

    private fun buildCameraUi() {
        val root = FrameLayout(this)
        val pv = PreviewView(this)
        preview = pv
        root.addView(pv, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        val hint = TextView(this).apply {
            text = "Point the camera at the QR code in the admin panel"
            setTextColor(0xFFFFFFFF.toInt()); setBackgroundColor(0x99000000.toInt()); setPadding(32, 160, 32, 32); gravity = Gravity.CENTER
        }
        root.addView(hint, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP))
        val bar = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER }
        val choose = Button(this).apply { text = "Choose image"; contentDescription = "Choose a picture that contains the QR code"; setOnClickListener { pickImage.launch("image/*") } }
        val cancel = Button(this).apply { text = "Cancel"; contentDescription = "Cancel scanning"; setOnClickListener { finish() } }
        bar.addView(choose); bar.addView(cancel)
        val lp = FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.BOTTOM)
        lp.bottomMargin = 96
        root.addView(bar, lp)
        setContentView(root)
    }

    /** Explains why the camera is needed before Android's own prompt. */
    private fun rationale() {
        AlertDialog.Builder(this)
            .setTitle("Allow camera access?")
            .setMessage("SuperChat uses the camera only to read the QR code of your login link. Nothing is recorded or sent anywhere.")
            .setPositiveButton("Continue") { _, _ -> askCamera.launch(Manifest.permission.CAMERA) }
            .setNegativeButton("Not now") { _, _ -> fail("The camera is needed to scan. Paste the link, or choose a picture of the QR code.") }
            .setOnCancelListener { fail("Scanning was cancelled.") }
            .show()
    }

    private fun fail(message: String) {
        Native.nativeLoginError(message)
        finish()
    }

    private fun startCamera() {
        val pv = preview ?: return
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            try {
                val provider = future.get()
                val prev = Preview.Builder().build().also { it.setSurfaceProvider(pv.surfaceProvider) }
                val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
                analysis.setAnalyzer(analysisExecutor) { proxy -> analyze(proxy) }
                provider.unbindAll()
                provider.bindToLifecycle(this, CameraSelector.DEFAULT_BACK_CAMERA, prev, analysis)
            } catch (e: Exception) {
                Log.e("atomic", "camera failed", e)
                fail("The camera is not available (${e.message}). Paste the link, or choose a picture of the QR code.")
            }
        }, ContextCompat.getMainExecutor(this))
    }

    private fun analyze(img: ImageProxy) {
        try {
            if (done) return
            val plane = img.planes[0]
            val buf = plane.buffer
            val bytes = ByteArray(buf.remaining()).also { buf.get(it) }
            val src = PlanarYUVLuminanceSource(bytes, plane.rowStride, img.height, 0, 0, img.width, img.height, false)
            val text = try { reader.decodeWithState(BinaryBitmap(HybridBinarizer(src))).text } catch (e: NotFoundException) { null } finally { reader.reset() }
            if (text != null) runOnUiThread { accept(text) }
        } catch (_: Exception) {
        } finally { img.close() }
    }

    private fun decodePicture(uri: Uri) {
        analysisExecutor.execute {
            val text = try {
                val opts = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, opts) }
                var sample = 1
                while (opts.outWidth / sample > 2200 || opts.outHeight / sample > 2200) sample *= 2
                val bmp: Bitmap? = contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, BitmapFactory.Options().apply { inSampleSize = sample }) }
                if (bmp == null) null else decodeBitmap(bmp)
            } catch (e: Exception) { Log.e("atomic", "image decode failed", e); null }
            runOnUiThread {
                if (text == null) fail("No QR code was found in that picture. Try a sharper, larger picture, or paste the link.") else accept(text)
            }
        }
    }

    private fun decodeBitmap(bmp: Bitmap): String? {
        val px = IntArray(bmp.width * bmp.height)
        bmp.getPixels(px, 0, bmp.width, 0, 0, bmp.width, bmp.height)
        val src = RGBLuminanceSource(bmp.width, bmp.height, px)
        val r = MultiFormatReader().apply { setHints(hints) }
        return try { r.decodeWithState(BinaryBitmap(HybridBinarizer(src))).text } catch (_: NotFoundException) {
            try { r.decodeWithState(BinaryBitmap(HybridBinarizer(InvertedLuminanceSource(src)))).text } catch (_: NotFoundException) { null }
        }
    }

    private fun accept(text: String) {
        if (done) return
        val ok = text.startsWith("superchat://") || text.startsWith("atomic-chat://") || ((text.startsWith("https://") || text.startsWith("http://")) && text.contains("/l/"))
        done = true
        if (ok) {
            Native.nativeLoginLink(text)
            finish()
        } else fail("That QR code is not a SuperChat login link.")
    }

    override fun onDestroy() {
        analysisExecutor.shutdown()
        super.onDestroy()
    }
}
