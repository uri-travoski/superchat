package app.superchat

import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject
import java.io.File

/**
 * Google push (FCM) receiver. NOT compiled unless `src/fcm/assets/fcm-config.json` exists (see build.gradle.kts), so
 * the default APK has no Google dependency. A push is a content-free wake: sync, then the Rust client posts its own
 * notification. The token is written to `fcm_token.txt`; the app registers it with the server from Settings > Push.
 */
class FcmService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        File(filesDir, "fcm_token.txt").writeText(token)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        Native.ensure(this)
        Native.nativeWake(filesDir.absolutePath)
    }

    companion object {
        /** Initialises Firebase from the bundled config and stores the current token. */
        fun init(ctx: android.content.Context) {
            if (FirebaseApp.getApps(ctx).isEmpty()) {
                val o = JSONObject(ctx.assets.open("fcm-config.json").bufferedReader().readText())
                FirebaseApp.initializeApp(
                    ctx,
                    FirebaseOptions.Builder().setProjectId(o.getString("project_id")).setApplicationId(o.getString("app_id")).setApiKey(o.getString("api_key")).setGcmSenderId(o.optString("sender_id")).build()
                )
            }
            FirebaseMessaging.getInstance().token.addOnSuccessListener { File(ctx.filesDir, "fcm_token.txt").writeText(it) }
        }
    }
}
