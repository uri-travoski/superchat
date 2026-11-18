package app.superchat

import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * N-01: subscribes to this device's ntfy topic (the Rust app writes `ntfy.json` after registering the topic with the
 * server). A pushed event only means "something happened": we wake the Rust client, which syncs and posts its own
 * notification. Nothing in the push carries message text.
 */
class NtfyListener(private val dir: File, private val onWake: () -> Unit) : Thread("ntfy") {
    @Volatile var stopping = false

    override fun run() {
        var lastCfg = ""
        while (!stopping) {
            val f = File(dir, "ntfy.json")
            val cfg = if (f.exists()) f.readText() else ""
            if (cfg.isEmpty()) { sleepQuietly(15_000); continue }
            try {
                val o = JSONObject(cfg)
                val url = URL("${o.getString("url")}/${o.getString("topic")}/json")
                val conn = url.openConnection() as HttpURLConnection
                conn.connectTimeout = 15_000
                conn.readTimeout = 120_000
                conn.inputStream.bufferedReader().use { r ->
                    // A change of settings is noticed at the next keep-alive (ntfy sends one every 45 s).
                    lastCfg = cfg
                    while (!stopping) {
                        val line = r.readLine() ?: break
                        if (JSONObject(line).optString("event") == "message") onWake()
                        val now = if (f.exists()) f.readText() else ""
                        if (now != lastCfg) break
                    }
                }
            } catch (e: Exception) {
                sleepQuietly(10_000)
            }
        }
    }

    private fun sleepQuietly(ms: Long) = try { sleep(ms) } catch (_: InterruptedException) {}
}
