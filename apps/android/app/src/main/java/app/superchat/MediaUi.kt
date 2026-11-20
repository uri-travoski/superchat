package app.superchat

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.view.View

/** Small line icons drawn with a Canvas (no resources, so the shrunk APK keeps them and every device renders the same). */
class GlyphView(ctx: Context, var kind: String, private val color: Int = 0xFFFFFFFF.toInt()) : View(ctx) {
    private val p = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = this@GlyphView.color; style = Paint.Style.STROKE; strokeCap = Paint.Cap.ROUND; strokeJoin = Paint.Join.ROUND }

    init { isClickable = true; isFocusable = true; contentDescription = kind }

    override fun onDraw(c: Canvas) {
        val s = minOf(width, height).toFloat()
        val u = s / 24f
        val ox = (width - s) / 2f
        val oy = (height - s) / 2f
        c.save(); c.translate(ox, oy)
        p.strokeWidth = 2f * u
        p.style = Paint.Style.STROKE
        when (kind) {
            "back" -> { val a = Path(); a.moveTo(19 * u, 12 * u); a.lineTo(5 * u, 12 * u); a.moveTo(12 * u, 19 * u); a.lineTo(5 * u, 12 * u); a.lineTo(12 * u, 5 * u); c.drawPath(a, p) }
            "close" -> { c.drawLine(6 * u, 6 * u, 18 * u, 18 * u, p); c.drawLine(18 * u, 6 * u, 6 * u, 18 * u, p) }
            "play" -> { p.style = Paint.Style.FILL; val a = Path(); a.moveTo(8 * u, 5 * u); a.lineTo(19 * u, 12 * u); a.lineTo(8 * u, 19 * u); a.close(); c.drawPath(a, p) }
            "pause" -> { p.style = Paint.Style.FILL; c.drawRoundRect(RectF(6 * u, 5 * u, 10 * u, 19 * u), u, u, p); c.drawRoundRect(RectF(14 * u, 5 * u, 18 * u, 19 * u), u, u, p) }
            "share" -> { val a = Path(); a.moveTo(4 * u, 12 * u); a.lineTo(4 * u, 20 * u); a.lineTo(20 * u, 20 * u); a.lineTo(20 * u, 12 * u); a.moveTo(16 * u, 6 * u); a.lineTo(12 * u, 2 * u); a.lineTo(8 * u, 6 * u); a.moveTo(12 * u, 2 * u); a.lineTo(12 * u, 15 * u); c.drawPath(a, p) }
            "save" -> { val a = Path(); a.moveTo(4 * u, 15 * u); a.lineTo(4 * u, 20 * u); a.lineTo(20 * u, 20 * u); a.lineTo(20 * u, 15 * u); a.moveTo(7 * u, 10 * u); a.lineTo(12 * u, 15 * u); a.lineTo(17 * u, 10 * u); a.moveTo(12 * u, 3 * u); a.lineTo(12 * u, 15 * u); c.drawPath(a, p) }
            "volume", "mute" -> {
                p.style = Paint.Style.FILL
                val a = Path(); a.moveTo(3 * u, 9 * u); a.lineTo(7 * u, 9 * u); a.lineTo(12 * u, 4 * u); a.lineTo(12 * u, 20 * u); a.lineTo(7 * u, 15 * u); a.lineTo(3 * u, 15 * u); a.close(); c.drawPath(a, p)
                p.style = Paint.Style.STROKE
                if (kind == "volume") { c.drawArc(RectF(10 * u, 8 * u, 18 * u, 16 * u), -50f, 100f, false, p); c.drawArc(RectF(8 * u, 4 * u, 22 * u, 20 * u), -50f, 100f, false, p) }
                else { c.drawLine(16 * u, 9 * u, 22 * u, 15 * u, p); c.drawLine(22 * u, 9 * u, 16 * u, 15 * u, p) }
            }
        }
        c.restore()
    }
}
