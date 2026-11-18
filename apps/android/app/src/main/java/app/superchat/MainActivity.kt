package app.superchat

import android.Manifest
import android.app.NativeActivity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle

/** Hosts the Slint UI (native activity). Everything else in this shell is Android-only glue. */
class MainActivity : NativeActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        Native.ensure(this)
        Native.activity = this
        SyncService.start(this)
        Native.nativeFontScale(resources.configuration.fontScale)
        Native.pushTimeZone()
        // Google push is compiled in only when a config was bundled (see build.gradle.kts).
        runCatching {
            val comp = Class.forName("app.superchat.FcmService").getField("Companion").get(null)!!
            comp.javaClass.getMethod("init", android.content.Context::class.java).invoke(comp, this)
        }
        handleIntent(intent)
    }

    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
        super.onConfigurationChanged(newConfig)
        Native.nativeFontScale(newConfig.fontScale)
    }

    override fun onResume() {
        super.onResume()
        Native.visible = true
        Native.nativeBackground(false)
        Native.pushTimeZone() // the zone (or daylight saving) may have changed while we were away
    }

    override fun onPause() {
        Native.visible = false
        Native.nativeBackground(true)
        super.onPause()
    }

    /** Every touch or key press postpones the App Lock idle timeout. */
    override fun onUserInteraction() {
        super.onUserInteraction()
        Native.nativeUserInput()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(i: Intent?) {
        Native.handleShare(this, i)
        // A superchat://login?... link (QR code scanned with the phone's camera app, or a tapped link).
        // Or the web form http(s)://<server>/l/<token> (the Rust side takes the server from the URL).
        if (i?.action == Intent.ACTION_VIEW) i.data?.takeIf {
            it.scheme == "superchat" || it.scheme == "atomic-chat" || ((it.scheme == "https" || it.scheme == "http") && (it.path ?: "").startsWith("/l/"))
        }?.let { Native.nativeLoginLink(it.toString()) }
        val conv = i?.getStringExtra("conv_id") ?: return
        Native.nativeOpenConversation(conv)
    }
}
