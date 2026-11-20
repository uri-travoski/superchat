package app.superchat

import android.animation.ValueAnimator
import android.app.Activity
import android.content.ContentValues
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.drawable.ColorDrawable
import android.media.ExifInterface
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.MediaStore
import android.view.Gravity
import android.view.GestureDetector
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.core.content.FileProvider
import org.json.JSONArray
import java.io.File
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

/**
 * 1.4.1 full-screen photo viewer: black background, pinch-zoom, double-tap zoom, pan, swipe left/right through the conversation's
 * pictures, swipe down or Back to close, top bar with the sender and date, Share and Save to the gallery (MediaStore).
 * Started by Native.viewMedia with a JSON list: [{"path","name","mime","sender","date"}], and the index to show.
 */
class ViewerActivity : Activity() {
    private class Item(val path: String, val name: String, val mime: String, val sender: String, val date: String)

    private val items = ArrayList<Item>()
    private var index = 0
    private lateinit var zoom: ZoomView
    private lateinit var who: TextView
    private lateinit var whenText: TextView
    private lateinit var bar: LinearLayout
    private lateinit var root: FrameLayout
    private var loadGen = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.setBackgroundDrawable(ColorDrawable(Color.BLACK))
        window.statusBarColor = Color.TRANSPARENT
        window.navigationBarColor = Color.BLACK
        window.addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS)
        val arr = JSONArray(intent.getStringExtra("items") ?: "[]")
        for (i in 0 until arr.length()) {
            val o = arr.getJSONObject(i)
            items.add(Item(o.getString("path"), o.optString("name"), o.optString("mime", "image/jpeg"), o.optString("sender"), o.optString("date")))
        }
        if (items.isEmpty()) { finish(); return }
        index = (savedInstanceState?.getInt("index") ?: intent.getIntExtra("index", 0)).coerceIn(0, items.size - 1)
        val d = resources.displayMetrics.density

        root = FrameLayout(this)
        root.setBackgroundColor(Color.BLACK)
        zoom = ZoomView()
        root.addView(zoom, FrameLayout.LayoutParams(-1, -1))

        bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(0x99000000.toInt())
        }
        fun btn(kind: String, desc: String, click: () -> Unit) = GlyphView(this, kind).apply {
            contentDescription = desc
            setOnClickListener { click() }
            setPadding((12 * d).toInt(), (12 * d).toInt(), (12 * d).toInt(), (12 * d).toInt())
        }
        bar.addView(btn("back", "Close") { finish() }, LinearLayout.LayoutParams((52 * d).toInt(), (52 * d).toInt()))
        val col = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        who = TextView(this).apply { setTextColor(Color.WHITE); textSize = 16f; setTypeface(typeface, android.graphics.Typeface.BOLD); maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END }
        whenText = TextView(this).apply { setTextColor(0xCCFFFFFF.toInt()); textSize = 12f; maxLines = 1 }
        col.addView(who); col.addView(whenText)
        bar.addView(col, LinearLayout.LayoutParams(0, -2, 1f))
        bar.addView(btn("share", "Share") { share() }, LinearLayout.LayoutParams((52 * d).toInt(), (52 * d).toInt()))
        bar.addView(btn("save", "Save to gallery") { save() }, LinearLayout.LayoutParams((52 * d).toInt(), (52 * d).toInt()))
        root.addView(bar, FrameLayout.LayoutParams(-1, -2, Gravity.TOP))
        // Keep the bar below the status bar.
        root.setOnApplyWindowInsetsListener { _, ins ->
            bar.setPadding(0, ins.systemWindowInsetTop, 0, 0)
            ins
        }
        setContentView(root)
        show(index, 0)
    }

    override fun onSaveInstanceState(out: Bundle) { super.onSaveInstanceState(out); out.putInt("index", index) }

    private fun show(i: Int, enterFrom: Int) {
        index = i
        val it = items[i]
        who.text = it.sender.ifEmpty { it.name }
        whenText.text = (if (items.size > 1) "${i + 1} of ${items.size}   " else "") + it.date
        val gen = ++loadGen
        Thread {
            val bmp = decode(it.path)
            runOnUiThread { if (gen == loadGen && !isDestroyed) zoom.setBitmap(bmp, enterFrom) }
        }.start()
    }

    private fun decode(path: String): Bitmap? = try {
        val o = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(path, o)
        var s = 1
        while (max(o.outWidth, o.outHeight) / s > 3072) s *= 2
        val b = BitmapFactory.decodeFile(path, BitmapFactory.Options().apply { inSampleSize = s })
        val rot = when (ExifInterface(path).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1)) { 6 -> 90f; 3 -> 180f; 8 -> 270f; else -> 0f }
        if (b != null && rot != 0f) Bitmap.createBitmap(b, 0, 0, b.width, b.height, Matrix().apply { postRotate(rot) }, true) else b
    } catch (e: Throwable) { null }

    private fun share() {
        val it = items[index]
        try {
            val uri = FileProvider.getUriForFile(this, "$packageName.fileprovider", File(it.path))
            val send = Intent(Intent.ACTION_SEND).setType(it.mime).putExtra(Intent.EXTRA_STREAM, uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            startActivity(Intent.createChooser(send, "Share").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
        } catch (e: Exception) { Toast.makeText(this, "Could not share this picture", Toast.LENGTH_SHORT).show() }
    }

    private fun save() {
        val it = items[index]
        val name = it.name.ifEmpty { File(it.path).name }
        Thread {
            val ok = try {
                val src = File(it.path)
                if (Build.VERSION.SDK_INT >= 29) {
                    val v = ContentValues().apply {
                        put(MediaStore.MediaColumns.DISPLAY_NAME, name)
                        put(MediaStore.MediaColumns.MIME_TYPE, it.mime)
                        put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/SuperChat")
                        put(MediaStore.MediaColumns.IS_PENDING, 1)
                    }
                    val uri = contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, v)!!
                    contentResolver.openOutputStream(uri)!!.use { o -> src.inputStream().use { i -> i.copyTo(o) } }
                    v.clear(); v.put(MediaStore.MediaColumns.IS_PENDING, 0)
                    contentResolver.update(uri, v, null, null)
                } else {
                    val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES), "SuperChat").apply { mkdirs() }
                    src.copyTo(File(dir, name), overwrite = true)
                    sendBroadcast(Intent(Intent.ACTION_MEDIA_SCANNER_SCAN_FILE).setData(android.net.Uri.fromFile(File(dir, name))))
                }
                true
            } catch (e: Throwable) { false }
            runOnUiThread { Toast.makeText(this, if (ok) "Saved to Pictures/SuperChat" else "Could not save the picture", Toast.LENGTH_SHORT).show() }
        }.start()
    }

    /** The picture with pinch-zoom, double-tap, pan, horizontal paging at 1x and swipe-down to close. */
    private inner class ZoomView : View(this@ViewerActivity) {
        private var bmp: Bitmap? = null
        private val m = Matrix()
        private val vals = FloatArray(9)
        private var base = 1f
        private val paint = Paint(Paint.FILTER_BITMAP_FLAG or Paint.ANTI_ALIAS_FLAG)
        private var dragX = 0f
        private var dragY = 0f
        private var mode = 0 // 0 none, 1 pan, 2 page, 3 dismiss
        private val slop = ViewConfiguration.get(context).scaledTouchSlop
        private var animating = false

        private val scaler = ScaleGestureDetector(context, object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            override fun onScale(d: ScaleGestureDetector): Boolean {
                val cur = scale()
                val f = (cur * d.scaleFactor).coerceIn(base, base * 6f) / cur
                m.postScale(f, f, d.focusX, d.focusY); fix(); invalidate(); return true
            }
        })
        private val taps = GestureDetector(context, object : GestureDetector.SimpleOnGestureListener() {
            override fun onDown(e: MotionEvent) = true
            override fun onSingleTapConfirmed(e: MotionEvent): Boolean { bar.visibility = if (bar.visibility == View.VISIBLE) View.GONE else View.VISIBLE; return true }
            override fun onDoubleTap(e: MotionEvent): Boolean {
                val target = if (scale() > base * 1.05f) base else base * 2.5f
                animateScale(target, e.x, e.y); return true
            }
        })

        fun setBitmap(b: Bitmap?, enterFrom: Int) {
            bmp = b
            resetMatrix()
            if (enterFrom != 0) { translationX = enterFrom * width.toFloat(); animate().translationX(0f).setDuration(160).start() }
            invalidate()
        }

        private fun scale(): Float { m.getValues(vals); return vals[Matrix.MSCALE_X] }

        private fun resetMatrix() {
            val b = bmp ?: return
            if (width == 0) return
            base = min(width / b.width.toFloat(), height / b.height.toFloat())
            m.reset(); m.postScale(base, base)
            m.postTranslate((width - b.width * base) / 2f, (height - b.height * base) / 2f)
        }

        override fun onSizeChanged(w: Int, h: Int, ow: Int, oh: Int) { resetMatrix() }

        /** Keeps the picture on screen: centred when smaller than the view, no empty edge when larger. */
        private fun fix() {
            val b = bmp ?: return
            m.getValues(vals)
            val s = vals[Matrix.MSCALE_X]
            val w = b.width * s; val h = b.height * s
            var tx = vals[Matrix.MTRANS_X]; var ty = vals[Matrix.MTRANS_Y]
            tx = if (w <= width) (width - w) / 2f else tx.coerceIn(width - w, 0f)
            ty = if (h <= height) (height - h) / 2f else ty.coerceIn(height - h, 0f)
            vals[Matrix.MTRANS_X] = tx; vals[Matrix.MTRANS_Y] = ty
            m.setValues(vals)
        }

        private fun animateScale(target: Float, fx: Float, fy: Float) {
            val from = scale()
            ValueAnimator.ofFloat(from, target).apply {
                duration = 180
                addUpdateListener { a -> val s = a.animatedValue as Float; val f = s / scale(); m.postScale(f, f, fx, fy); fix(); invalidate() }
                start()
            }
        }

        override fun onDraw(c: Canvas) {
            val b = bmp ?: return
            c.drawBitmap(b, m, paint)
        }

        override fun onTouchEvent(e: MotionEvent): Boolean {
            if (animating) return true
            scaler.onTouchEvent(e)
            taps.onTouchEvent(e)
            if (scaler.isInProgress || e.pointerCount > 1) { mode = 0; return true }
            when (e.actionMasked) {
                MotionEvent.ACTION_DOWN -> { dragX = e.x; dragY = e.y; mode = 0 }
                MotionEvent.ACTION_MOVE -> {
                    val dx = e.x - dragX; val dy = e.y - dragY
                    val zoomed = scale() > base * 1.02f
                    if (mode == 0 && (abs(dx) > slop || abs(dy) > slop)) {
                        mode = if (zoomed) 1 else if (abs(dx) > abs(dy)) 2 else if (dy > 0) 3 else 0
                    }
                    when (mode) {
                        1 -> { m.postTranslate(e.x - lastX, e.y - lastY); fix(); invalidate() }
                        2 -> translationX = dx
                        3 -> { translationY = max(0f, dy); root.setBackgroundColor(Color.argb((255 * (1f - min(1f, dy / (height * 0.6f)))).toInt(), 0, 0, 0)) }
                    }
                }
                MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                    val dx = e.x - dragX; val dy = e.y - dragY
                    when (mode) {
                        2 -> {
                            val next = if (dx < -width * 0.22f) index + 1 else if (dx > width * 0.22f) index - 1 else index
                            if (next != index && next in items.indices) {
                                animating = true
                                val dir = if (next > index) -1 else 1
                                animate().translationX(dir * width.toFloat()).setDuration(120).withEndAction {
                                    animating = false; translationX = 0f
                                    bmp = null; invalidate()
                                    show(next, -dir)
                                }.start()
                            } else animate().translationX(0f).setDuration(120).start()
                        }
                        3 -> if (dy > height * 0.18f) finish() else { animate().translationY(0f).setDuration(120).start(); root.setBackgroundColor(Color.BLACK) }
                    }
                    mode = 0
                }
            }
            lastX = e.x; lastY = e.y
            return true
        }
        private var lastX = 0f
        private var lastY = 0f
    }
}
