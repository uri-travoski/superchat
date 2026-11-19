package app.superchat

import android.Manifest
import android.app.NativeActivity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.KeyEvent

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
        runCatching {
            Native.nativeVersionCode(
                if (Build.VERSION.SDK_INT >= 28) packageManager.getPackageInfo(packageName, 0).longVersionCode.toInt()
                else @Suppress("DEPRECATION") packageManager.getPackageInfo(packageName, 0).versionCode
            )
        }
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
        Installer.resumePending(this) // the user may have just allowed "Install unknown apps"
        Native.pushTimeZone() // the zone (or daylight saving) may have changed while we were away
    }

    override fun onPause() {
        Native.visible = false
        Native.nativeBackground(true)
        super.onPause()
    }

    /**
     * System Back goes to the Rust UI's one Back stack (popup, sheet, page, chat). Only when nothing is left to close does the
     * app leave: the task moves to the background, which is the standard Android behaviour for a root screen.
     * (Legacy Back dispatch is selected in the manifest, so there is no predictive animation to get out of sync.)
     */
    private fun handleBack() {
        if (!Native.nativeBack()) moveTaskToBack(true)
    }

    @Deprecated("Back is routed through the Rust UI")
    override fun onBackPressed() = handleBack()

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (event.keyCode == KeyEvent.KEYCODE_BACK) {
            if (event.action == KeyEvent.ACTION_UP && !event.isCanceled) handleBack()
            return true
        }
        return super.dispatchKeyEvent(event)
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
