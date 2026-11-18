package app.superchat

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build

/** adb shell am broadcast -a app.superchat.TEST_NOTIFY --es conv_id abc --es title Dan --es text hi */
class TestNotifyReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val conv = intent.getStringExtra("conv_id") ?: "unknown"
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(NotificationChannel("messages", "Messages", NotificationManager.IMPORTANCE_HIGH))
        }
        val open = PendingIntent.getActivity(
            ctx, conv.hashCode(),
            Intent(ctx, MainActivity::class.java).putExtra("conv_id", conv).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        nm.notify(
            conv.hashCode(),
            android.app.Notification.Builder(ctx, "messages")
                .setContentTitle(intent.getStringExtra("title") ?: "SuperChat")
                .setContentText(intent.getStringExtra("text") ?: "New message")
                .setSmallIcon(android.R.drawable.stat_notify_chat)
                .setContentIntent(open)
                .setAutoCancel(true)
                .build()
        )
    }
}
