package app.superchat

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import java.io.File

/**
 * Installs an APK that the Rust side downloaded and verified (filesDir/updates). Uses a PackageInstaller session, which tells
 * us how it ended (the system shows its own confirmation). Results go back to Rust as `nativeInstallResult(kind, message)`
 * with kind "cancelled", "permission" or "failed"; success needs no report because Android restarts the app.
 *
 * "Install unknown apps" is a per-app permission: when it is off, the settings page for this app opens and the install
 * continues from [resumePending] when the user comes back with it on.
 */
object Installer {
    const val ACTION_RESULT = "app.superchat.INSTALL_RESULT"

    /** The file waiting for the "Install unknown apps" permission. */
    @Volatile private var pending: String? = null

    private fun canInstall(ctx: Context) = Build.VERSION.SDK_INT < 26 || ctx.packageManager.canRequestPackageInstalls()

    @JvmStatic fun install(act: Activity, path: String) {
        val f = File(path)
        if (!f.isFile) {
            Native.nativeInstallResult("failed", "The downloaded update file is missing. Download it again.")
            return
        }
        if (!canInstall(act)) {
            pending = path
            Native.nativeInstallResult("permission", "")
            try {
                act.startActivity(
                    Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${act.packageName}"))
                )
            } catch (e: Exception) {
                pending = null
                Native.nativeInstallResult("failed", "Android's \"Install unknown apps\" settings could not be opened. Open Settings > Apps > SuperChat > Install unknown apps and allow it.")
            }
            return
        }
        startSession(act, f)
    }

    /** Called from MainActivity.onResume: the user may have just allowed installs. */
    fun resumePending(act: Activity) {
        val p = pending ?: return
        if (!canInstall(act)) return
        pending = null
        startSession(act, File(p))
    }

    private fun startSession(act: Activity, f: File) {
        try {
            val pi = act.packageManager.packageInstaller
            val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
            params.setSize(f.length())
            val id = pi.createSession(params)
            pi.openSession(id).use { s ->
                f.inputStream().use { input ->
                    s.openWrite("superchat.apk", 0, f.length()).use { out ->
                        input.copyTo(out)
                        s.fsync(out)
                    }
                }
                val cb = PendingIntent.getBroadcast(
                    act, id,
                    Intent(ACTION_RESULT).setPackage(act.packageName),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE
                )
                s.commit(cb.intentSender)
            }
        } catch (e: Exception) {
            // No PackageInstaller or the session could not be written: fall back to the plain install intent.
            viewIntent(act, f)
        }
    }

    private fun viewIntent(act: Activity, f: File) {
        try {
            val uri = FileProvider.getUriForFile(act, "${act.packageName}.fileprovider", f)
            val i = Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, "application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            if (i.resolveActivity(act.packageManager) == null) {
                Native.nativeInstallResult("failed", "This device has no app installer that can open the update. Download it with a browser from the releases page instead.")
                return
            }
            act.startActivity(i)
        } catch (e: Exception) {
            Native.nativeInstallResult("failed", "The installer could not be started: ${e.message}")
        }
    }

    /** Receives the PackageInstaller session status. */
    class ResultReceiver : BroadcastReceiver() {
        override fun onReceive(ctx: Context, intent: Intent) {
            val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
            val msg = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: ""
            when (status) {
                PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                    @Suppress("DEPRECATION")
                    val next = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
                    if (next != null) {
                        try {
                            ctx.startActivity(next.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                        } catch (e: Exception) {
                            Native.nativeInstallResult("failed", "The system installer could not be opened: ${e.message}")
                        }
                    }
                }
                PackageInstaller.STATUS_SUCCESS -> {}
                PackageInstaller.STATUS_FAILURE_ABORTED -> Native.nativeInstallResult("cancelled", msg)
                PackageInstaller.STATUS_FAILURE_INCOMPATIBLE ->
                    Native.nativeInstallResult("failed", SIGNATURE_MESSAGE)
                PackageInstaller.STATUS_FAILURE_CONFLICT ->
                    Native.nativeInstallResult(
                        "failed",
                        if (looksLikeSignature(msg)) SIGNATURE_MESSAGE else "Android reports a conflict with the installed app: $msg"
                    )
                PackageInstaller.STATUS_FAILURE_STORAGE ->
                    Native.nativeInstallResult("failed", "There is not enough free storage to install the update. Free some space and try again.")
                PackageInstaller.STATUS_FAILURE_BLOCKED ->
                    Native.nativeInstallResult("failed", "Android (or the device's policy) blocked this install: $msg")
                PackageInstaller.STATUS_FAILURE_INVALID ->
                    Native.nativeInstallResult("failed", "Android says the downloaded file is not a valid app package. Download it again.")
                else ->
                    Native.nativeInstallResult(
                        "failed",
                        if (looksLikeSignature(msg)) SIGNATURE_MESSAGE else "The install failed: $msg"
                    )
            }
        }

        private fun looksLikeSignature(m: String) =
            m.contains("signatures do not match", true) || m.contains("UPDATE_INCOMPATIBLE", true) || m.contains("signature", true)
    }

    const val SIGNATURE_MESSAGE =
        "This update was signed with a different key than the installed app, so Android refuses to update it. " +
            "Uninstall SuperChat and install the new version (sign in again afterwards); later updates will then work."
}
