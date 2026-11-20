package app.superchat

import android.app.Activity
import android.graphics.Color
import android.graphics.drawable.ColorDrawable
import android.media.MediaPlayer
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.SeekBar
import android.widget.TextView
import android.widget.Toast
import android.widget.VideoView
import java.io.File

/**
 * 1.4.1 full-screen video player: black screen, play/pause, a seek bar with the time, mute, Back closes. Tapping the picture shows
 * or hides the controls. Started by Native.playVideo(path, title).
 */
class VideoActivity : Activity() {
    private lateinit var video: VideoView
    private lateinit var seek: SeekBar
    private lateinit var time: TextView
    private lateinit var playBtn: GlyphView
    private lateinit var muteBtn: GlyphView
    private lateinit var top: LinearLayout
    private lateinit var bottom: LinearLayout
    private var player: MediaPlayer? = null
    private var muted = false
    private var dragging = false
    private val h = Handler(Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() {
            if (!dragging && video.duration > 0) { seek.progress = video.currentPosition; time.text = fmt(video.currentPosition) + " / " + fmt(video.duration) }
            playBtn.kind = if (video.isPlaying) "pause" else "play"; playBtn.invalidate()
            h.postDelayed(this, 250)
        }
    }

    private fun fmt(ms: Int): String { val s = ms / 1000; return "%d:%02d".format(s / 60, s % 60) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.setBackgroundDrawable(ColorDrawable(Color.BLACK))
        window.statusBarColor = Color.BLACK
        window.navigationBarColor = Color.BLACK
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val path = intent.getStringExtra("path") ?: run { finish(); return }
        val d = resources.displayMetrics.density
        val root = FrameLayout(this).apply { setBackgroundColor(Color.BLACK) }
        video = VideoView(this)
        root.addView(video, FrameLayout.LayoutParams(-1, -1, Gravity.CENTER))
        root.setOnClickListener { toggleControls() }

        top = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; setBackgroundColor(0x99000000.toInt()) }
        top.addView(GlyphView(this, "back").apply { contentDescription = "Close"; setPadding((12 * d).toInt(), (12 * d).toInt(), (12 * d).toInt(), (12 * d).toInt()); setOnClickListener { finish() } }, LinearLayout.LayoutParams((52 * d).toInt(), (52 * d).toInt()))
        top.addView(TextView(this).apply { text = intent.getStringExtra("title") ?: File(path).name; setTextColor(Color.WHITE); textSize = 16f; maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END }, LinearLayout.LayoutParams(0, -2, 1f))
        root.addView(top, FrameLayout.LayoutParams(-1, -2, Gravity.TOP))

        bottom = LinearLayout(this).apply { gravity = Gravity.CENTER_VERTICAL; setBackgroundColor(0x99000000.toInt()); setPadding((6 * d).toInt(), 0, (10 * d).toInt(), 0) }
        playBtn = GlyphView(this, "pause").apply { contentDescription = "Play or pause"; setPadding((13 * d).toInt(), (13 * d).toInt(), (13 * d).toInt(), (13 * d).toInt()); setOnClickListener { if (video.isPlaying) video.pause() else video.start() } }
        bottom.addView(playBtn, LinearLayout.LayoutParams((56 * d).toInt(), (56 * d).toInt()))
        seek = SeekBar(this).apply {
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(s: SeekBar, p: Int, user: Boolean) { if (user) time.text = fmt(p) + " / " + fmt(video.duration) }
                override fun onStartTrackingTouch(s: SeekBar) { dragging = true }
                override fun onStopTrackingTouch(s: SeekBar) { dragging = false; video.seekTo(s.progress) }
            })
        }
        bottom.addView(seek, LinearLayout.LayoutParams(0, -2, 1f))
        time = TextView(this).apply { text = "0:00 / 0:00"; setTextColor(Color.WHITE); textSize = 12f; setPadding((8 * d).toInt(), 0, (4 * d).toInt(), 0) }
        bottom.addView(time)
        muteBtn = GlyphView(this, "volume").apply { contentDescription = "Mute"; setPadding((13 * d).toInt(), (13 * d).toInt(), (13 * d).toInt(), (13 * d).toInt()); setOnClickListener { setMuted(!muted) } }
        bottom.addView(muteBtn, LinearLayout.LayoutParams((56 * d).toInt(), (56 * d).toInt()))
        root.addView(bottom, FrameLayout.LayoutParams(-1, -2, Gravity.BOTTOM))
        root.setOnApplyWindowInsetsListener { _, ins ->
            top.setPadding(0, ins.systemWindowInsetTop, 0, 0)
            bottom.setPadding((6 * d).toInt(), 0, (10 * d).toInt(), ins.systemWindowInsetBottom)
            ins
        }
        setContentView(root)

        video.setOnPreparedListener { mp ->
            player = mp
            mp.setVolume(if (muted) 0f else 1f, if (muted) 0f else 1f)
            seek.max = mp.duration
            time.text = "0:00 / " + fmt(mp.duration)
            video.start()
            h.removeCallbacks(tick); h.post(tick)
        }
        video.setOnCompletionListener { seek.progress = seek.max; playBtn.kind = "play"; playBtn.invalidate(); showControls() }
        video.setOnErrorListener { _, _, _ -> Toast.makeText(this, "This video cannot be played", Toast.LENGTH_LONG).show(); finish(); true }
        video.setVideoURI(Uri.fromFile(File(path)))
    }

    private fun setMuted(m: Boolean) {
        muted = m
        player?.setVolume(if (m) 0f else 1f, if (m) 0f else 1f)
        muteBtn.kind = if (m) "mute" else "volume"; muteBtn.contentDescription = if (m) "Unmute" else "Mute"; muteBtn.invalidate()
    }

    private fun showControls() { top.visibility = View.VISIBLE; bottom.visibility = View.VISIBLE }
    private fun toggleControls() { val v = if (top.visibility == View.VISIBLE) View.GONE else View.VISIBLE; top.visibility = v; bottom.visibility = v }

    override fun onPause() { super.onPause(); if (::video.isInitialized && video.isPlaying) video.pause() }
    override fun onDestroy() { h.removeCallbacksAndMessages(null); super.onDestroy() }
}
