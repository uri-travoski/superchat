package app.superchat

import android.Manifest
import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.io.File
import java.security.KeyStore
import java.util.TimeZone
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Bridge between the Rust app (libatomic_android.so) and Android. Rust calls the @JvmStatic methods through JNI
 * (notifications, audio, Keystore); Kotlin calls the `external` functions to hand data to Rust.
 */
object Native {
    init {
        System.loadLibrary("atomic_android")
    }

    @JvmStatic external fun init()
    @JvmStatic external fun nativeOpenConversation(convId: String)
    @JvmStatic external fun nativeFontScale(scale: Float)
    @JvmStatic external fun nativeWake(dataDir: String)
    @JvmStatic external fun nativeTzOffset(secs: Int)
    @JvmStatic external fun nativeShare(text: String, files: String)
    @JvmStatic external fun nativeLoginLink(link: String)
    @JvmStatic external fun nativeUserInput()
    @JvmStatic external fun nativeBackground(background: Boolean)
    /** System Back: true when the UI closed something (popup, page, chat); false when nothing is left to close. */
    @JvmStatic external fun nativeBack(): Boolean
    /** Files from the picker: paths and one JSON line of media details per path, both newline separated. */
    @JvmStatic external fun nativePicked(paths: String, metas: String)
    /** Tells the login screen why a scan could not be done (shown as text, never silent). */
    @JvmStatic external fun nativeLoginError(message: String)
    /** What the system installer said about a downloaded update: kind is "cancelled", "permission" or "failed". */
    @JvmStatic external fun nativeInstallResult(kind: String, message: String)
    /** This build's versionCode, for the About page. */
    @JvmStatic external fun nativeVersionCode(code: Int)
    /** Build.SUPPORTED_ABIS, best first, comma separated: the update check downloads the APK built for the first one it ships. */
    @JvmStatic external fun nativeAbis(abis: String)

    private lateinit var app: Context
    private var ready = false

    /** True while the Activity is on screen: notifications are then not needed. */
    @Volatile var visible = false

    @Synchronized
    fun ensure(ctx: Context) {
        if (ready) return
        app = ctx.applicationContext
        init()
        ready = true
    }

    fun pushTimeZone() {
        nativeTzOffset(TimeZone.getDefault().getOffset(System.currentTimeMillis()) / 1000)
    }

    // ---- update checks (called from Rust) -------------------------------------------------------------------------------

    /** Hands a downloaded and verified APK (filesDir/updates) to the system installer. */
    @JvmStatic fun installApkPath(path: String): String {
        val a = activity
        if (a == null) {
            nativeInstallResult("failed", "SuperChat is not on screen. Open it and try again.")
            return "no-activity"
        }
        a.runOnUiThread { Installer.install(a, path) }
        return "ok"
    }

    /** Opens a web page in the browser; false when the device has no browser. */
    @JvmStatic fun openUrl(url: String): Boolean {
        val a = activity ?: return false
        return try {
            a.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        } catch (e: Exception) {
            false
        }
    }

    // ---- N-01: local notification for a new message (called from Rust) ------------------------------------------

    /**
     * N-04: [sound] is "default" (the system sound), "none" (silent) or a bundled id (res/raw/sound_<id>). Android fixes a
     * channel's sound once it exists, so each sound gets its own channel; the user can still change any of them in
     * Android's notification settings. [unread] is the launcher badge number (0 hides it).
     */
    @JvmStatic fun notifyMessage(title: String, body: String, convId: String, sound: String, unread: Int) {
        if (visible) return
        val nm = app.getSystemService(NotificationManager::class.java)
        val channel = if (sound == "default") "messages" else "messages_$sound"
        if (Build.VERSION.SDK_INT >= 26) {
            val label = when (sound) { "default" -> "Messages"; "none" -> "Messages (silent)"; else -> "Messages (${sound.replaceFirstChar { it.uppercase() }})" }
            val ch = NotificationChannel(channel, label, NotificationManager.IMPORTANCE_HIGH)
            ch.setShowBadge(true)
            when (sound) {
                "default" -> {}
                "none" -> ch.setSound(null, null)
                else -> {
                    val res = app.resources.getIdentifier("sound_$sound", "raw", app.packageName)
                    if (res != 0) ch.setSound(
                        Uri.parse("android.resource://${app.packageName}/$res"),
                        android.media.AudioAttributes.Builder().setUsage(android.media.AudioAttributes.USAGE_NOTIFICATION).setContentType(android.media.AudioAttributes.CONTENT_TYPE_SONIFICATION).build()
                    )
                }
            }
            nm.createNotificationChannel(ch)
        }
        val open = PendingIntent.getActivity(
            app, convId.hashCode(),
            Intent(app, MainActivity::class.java).putExtra("conv_id", convId).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val b = android.app.Notification.Builder(app, channel)
            .setContentTitle(title)
            .setContentText(body)
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentIntent(open)
            .setAutoCancel(true)
        if (unread > 0) b.setNumber(unread)
        nm.notify(convId.hashCode(), b.build())
    }

    // ---- X-02: secrets encrypted with a Keystore key ---------------------------------------------------------------

    private const val KEY_ALIAS = "atomic_chat_secrets"

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val g = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        g.init(
            KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return g.generateKey()
    }

    @JvmStatic fun encrypt(plain: String): String? = try {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, key())
        Base64.encodeToString(c.iv + c.doFinal(plain.toByteArray()), Base64.NO_WRAP)
    } catch (e: Exception) { null }

    @JvmStatic fun decrypt(text: String): String? = try {
        val raw = Base64.decode(text, Base64.NO_WRAP)
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, raw.copyOfRange(0, 12)))
        String(c.doFinal(raw, 12, raw.size - 12))
    } catch (e: Exception) { null }

    // ---- QR login: opens the camera scanner (called from Rust) ------------------------------------------------------

    @JvmStatic fun scanQr() = openScanner(false)

    /** "Choose image with QR": the photo picker, then ZXing decodes the picture. */
    @JvmStatic fun scanImage() = openScanner(true)

    private fun openScanner(fromImage: Boolean) {
        val a = activity
        if (a == null) {
            nativeLoginError("The scanner could not start (the app window is not ready). Paste the link instead.")
            return
        }
        a.runOnUiThread {
            try {
                a.startActivity(Intent(a, ScanActivity::class.java).putExtra("image", fromImage))
            } catch (e: Exception) {
                android.util.Log.e("atomic", "scanner failed to start", e)
                nativeLoginError("The scanner could not start: ${e.message}. Paste the link instead.")
            }
        }
    }

    /** M-31: opens the file picker ("files"), the photo picker ("gallery") or the image picker ("image"); `limit` is the upload limit in bytes. */
    @JvmStatic fun pickMedia(mode: String, limit: Long) {
        val a = activity ?: return
        a.runOnUiThread {
            try {
                a.startActivity(Intent(a, MediaActivity::class.java).putExtra("mode", mode).putExtra("limit", limit))
            } catch (e: Exception) {
                android.util.Log.e("atomic", "media screen failed to start", e)
            }
        }
    }

    /** Plain text on the clipboard (the login screen's Paste button); empty when there is none. */
    @JvmStatic fun clipboardText(@Suppress("UNUSED_PARAMETER") unused: String): String = try {
        val cm = app.getSystemService(Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
        cm.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(app)?.toString() ?: ""
    } catch (e: Exception) { "" }

    // ---- M-30: voice messages ---------------------------------------------------------------------------------------

    private var player: MediaPlayer? = null
    private var recorder: MediaRecorder? = null
    private var recordPath: String? = null
    var activity: Activity? = null

    @JvmStatic fun play(path: String): String {
        stop()
        return try {
            val p = MediaPlayer()
            p.setDataSource(path)
            p.setOnCompletionListener { stop() }
            p.prepare()
            p.start()
            player = p
            "ok"
        } catch (e: Exception) { "" }
    }

    @JvmStatic fun stop() {
        player?.let { try { it.stop() } catch (_: Exception) {}; it.release() }
        player = null
        paused = false
    }

    @JvmStatic fun isPlaying(): Boolean = try { player?.isPlaying == true } catch (_: Exception) { false }

    // Voice player: pause, seek and speed (1x, 1.5x, 2x). A paused player is still "active" so the UI keeps its place.
    private var paused = false
    private var speed = 1f

    @JvmStatic fun playFrom(path: String, startMs: Int, spd: Float): String {
        stop()
        return try {
            val p = MediaPlayer()
            p.setDataSource(path)
            p.setOnCompletionListener { stop() }
            p.prepare()
            if (startMs > 0) p.seekTo(startMs)
            speed = spd
            p.start()
            if (spd != 1f) p.playbackParams = p.playbackParams.setSpeed(spd)
            player = p
            "ok"
        } catch (e: Exception) { "" }
    }

    @JvmStatic fun pausePlayback() {
        try { player?.let { if (it.isPlaying) { it.pause(); paused = true } } } catch (_: Exception) {}
    }

    @JvmStatic fun resumePlayback(): Boolean = try {
        val p = player
        if (p != null && paused) {
            p.start()
            if (speed != 1f) p.playbackParams = p.playbackParams.setSpeed(speed)
            paused = false
            true
        } else false
    } catch (e: Exception) { false }

    @JvmStatic fun seekTo(ms: Int) {
        try { player?.seekTo(ms) } catch (_: Exception) {}
    }

    @JvmStatic fun setPlaybackSpeed(spd: Float) {
        speed = spd
        // Changing the speed of a paused player would start it; the speed is applied on resume instead.
        try { player?.let { if (it.isPlaying) it.playbackParams = it.playbackParams.setSpeed(spd) } } catch (_: Exception) {}
    }

    @JvmStatic fun position(): Int = try { player?.currentPosition ?: -1 } catch (e: Exception) { -1 }

    @JvmStatic fun isActive(): Boolean = player != null

    @JvmStatic fun recordStart(): Boolean {
        if (app.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            activity?.requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), 2)
            return false
        }
        return try {
            val f = File(app.cacheDir, "voice-message-${System.currentTimeMillis() / 1000}.m4a")
            val r = if (Build.VERSION.SDK_INT >= 31) MediaRecorder(app) else @Suppress("DEPRECATION") MediaRecorder()
            r.setAudioSource(MediaRecorder.AudioSource.MIC)
            r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
            r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            r.setAudioSamplingRate(16000)
            r.setAudioEncodingBitRate(32000)
            r.setOutputFile(f.absolutePath)
            r.prepare()
            r.start()
            recorder = r
            recordPath = f.absolutePath
            true
        } catch (e: Exception) { recorder?.release(); recorder = null; false }
    }

    @JvmStatic fun recordStop(): String? {
        val r = recorder ?: return null
        recorder = null
        return try {
            r.stop(); r.release()
            recordPath
        } catch (e: Exception) { try { r.release() } catch (_: Exception) {}; null }
    }

    // ---- E-06: share target ---------------------------------------------------------------------------------------------

    fun handleShare(ctx: Context, i: Intent?) {
        if (i == null) return
        if (i.action != Intent.ACTION_SEND && i.action != Intent.ACTION_SEND_MULTIPLE) return
        val text = i.getStringExtra(Intent.EXTRA_TEXT) ?: ""
        val uris = ArrayList<Uri>()
        if (i.action == Intent.ACTION_SEND) {
            (if (Build.VERSION.SDK_INT >= 33) i.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java) else @Suppress("DEPRECATION") i.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))?.let { uris.add(it) }
        } else {
            (if (Build.VERSION.SDK_INT >= 33) i.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java) else @Suppress("DEPRECATION") i.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM))?.let { uris.addAll(it) }
        }
        val dir = File(ctx.cacheDir, "shared").apply { mkdirs() }
        val paths = uris.mapNotNull { copyUri(ctx, it, dir) }
        nativeShare(text, paths.joinToString("\n"))
    }

    private fun copyUri(ctx: Context, u: Uri, dir: File): String? = try {
        var name = "shared-${System.currentTimeMillis()}"
        ctx.contentResolver.query(u, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { if (it.moveToFirst()) name = it.getString(0) ?: name }
        val out = File(dir, name.replace('/', '_'))
        ctx.contentResolver.openInputStream(u)?.use { input -> out.outputStream().use { input.copyTo(it) } }
        out.absolutePath
    } catch (e: Exception) { null }
}
