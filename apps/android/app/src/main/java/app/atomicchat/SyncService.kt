package app.atomicchat

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * X-06: foreground service with a low-priority persistent notification that keeps the process
 * (and the Rust realtime connection) alive while the screen is off.
 */
class SyncService : Service() {
    companion object {
        @JvmStatic external fun nativeStart(dataDir: String)

        private var started = false
        private var ntfy: NtfyListener? = null

        fun start(ctx: Context) {
            val i = Intent(ctx, SyncService::class.java)
            if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(i) else ctx.startService(i)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val nm = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(
                NotificationChannel("sync", "Connection", NotificationManager.IMPORTANCE_MIN).apply {
                    description = "Keeps Atomic Chat connected"
                    setShowBadge(false)
                }
            )
        }
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val n: Notification = Notification.Builder(this, "sync")
            .setContentTitle("Atomic Chat")
            .setContentText("Connected")
            .setSmallIcon(android.R.drawable.stat_notify_chat)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(1, n)
        }
        if (!started) {
            started = true
            Native.ensure(this)
            nativeStart(filesDir.absolutePath)
            ntfy = NtfyListener(filesDir) { Native.nativeWake(filesDir.absolutePath) }.also { it.isDaemon = true; it.start() }
        }
        return START_STICKY
    }
}
