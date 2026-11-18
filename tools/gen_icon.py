#!/usr/bin/env python3
"""Generates every SuperChat icon from one geometry. Needs only ImageMagick (`magick`).

* assets/icon/superchat.svg   the master artwork (real gradients; open it in any browser or editor)
* assets/icon/superchat-<n>.png, superchat.ico, Android adaptive foreground, admin favicon/logo

ImageMagick's built-in SVG renderer cannot draw gradients, so the PNGs are composited from the same
shapes (flat SVG masks + magick gradients). Run from the repo root:  python3 tools/gen_icon.py
"""
import math, os, subprocess, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MINT = "#0DBD8B"
BUBBLE = "M382,262 H642 A150,150 0 0 1 792,412 V520 A150,150 0 0 1 642,670 H548 L372,806 C362,813 350,806 354,794 L382,670 A150,150 0 0 1 232,520 V412 A150,150 0 0 1 382,262 Z"
DOTS = [(402, 466), (512, 466), (622, 466)]
DOT_R = 38
SHIFT = -19  # vertical shift so the bubble plus its tail is optically centred


def squircle(r, n=5.0, steps=240):
    pts = []
    for i in range(steps):
        t = 2 * math.pi * i / steps
        c, s = math.cos(t), math.sin(t)
        pts.append(f"{512 + r * math.copysign(abs(c) ** (2 / n), c):.1f},{512 + r * math.copysign(abs(s) ** (2 / n), s):.1f}")
    return "M" + " L".join(pts) + " Z"


def sh(*a):
    subprocess.run(["magick", *a], check=True)


def master_svg():
    dots = "".join(f'<circle cx="{x}" cy="{y}" r="{DOT_R}" fill="#0b1a1a" fill-opacity="0.82"/>' for x, y in DOTS)
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
<title>SuperChat</title>
<defs>
<radialGradient id="glow" gradientUnits="userSpaceOnUse" cx="512" cy="400" r="640"><stop offset="0" stop-color="#17413a"/><stop offset="0.55" stop-color="#0d1a1c"/><stop offset="1" stop-color="#07090d"/></radialGradient>
<radialGradient id="halo" gradientUnits="userSpaceOnUse" cx="512" cy="540" r="400"><stop offset="0" stop-color="{MINT}" stop-opacity="0.38"/><stop offset="1" stop-color="{MINT}" stop-opacity="0"/></radialGradient>
<linearGradient id="bub" gradientUnits="userSpaceOnUse" x1="300" y1="262" x2="740" y2="800"><stop offset="0" stop-color="#ffffff"/><stop offset="0.45" stop-color="#e9fbf5"/><stop offset="1" stop-color="{MINT}"/></linearGradient>
</defs>
<path d="{squircle(512)}" fill="url(#glow)"/>
<path d="{squircle(506)}" fill="none" stroke="#ffffff" stroke-opacity="0.12" stroke-width="6"/>
<g transform="translate(0,{SHIFT})"><ellipse cx="512" cy="540" rx="400" ry="380" fill="url(#halo)"/>
<path d="{BUBBLE}" fill="url(#bub)"/>{dots}</g>
</svg>
'''


def flat_svg(path, body):
    open(path, "w").write(f'<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">{body}</svg>')


def build(tmp):
    t = lambda n: os.path.join(tmp, n)
    flat_svg(t("sq.svg"), f'<path d="{squircle(512)}" fill="#fff"/>')
    flat_svg(t("rim.svg"), f'<path d="{squircle(506)}" fill="none" stroke="#fff" stroke-width="6"/>')
    flat_svg(t("bub.svg"), f'<g transform="translate(0,{SHIFT})"><path d="{BUBBLE}" fill="#fff"/></g>')
    flat_svg(t("dots.svg"), '<g transform="translate(0,%d)">%s</g>' % (SHIFT, "".join(f'<circle cx="{x}" cy="{y}" r="{DOT_R}" fill="#fff"/>' for x, y in DOTS)))
    for n in ("sq", "rim", "bub", "dots"):  # alpha masks
        sh("-background", "none", t(n + ".svg"), "-alpha", "extract", t(n + ".mask.png"))
    # background glow: radial gradient, centre at (512,400)
    sh("-size", "1400x1400", "radial-gradient:#1a463e-#07090d", "-crop", "1024x1024+188+300", "+repage", t("glow.png"))
    # mint halo behind the bubble, added with screen
    sh("-size", "800x800", "radial-gradient:#0DBD8B-black", "-evaluate", "multiply", "0.38", "-background", "black", "-gravity", "northwest",
       "-extent", "1024x1024-112+%d" % (140 + SHIFT), t("halo.png"))
    sh(t("glow.png"), t("halo.png"), "-compose", "screen", "-composite", t("bg.png"))
    # bubble fill: white -> mint along a diagonal
    # gradient tilted ~27 degrees: white in the upper left, mint towards the tail (bottom)
    sh("-size", "1800x1800", "gradient:black-white", "-background", "black", "-rotate", "26.5", "-gravity", "center",
       "-crop", "1024x1024+0+0", "+repage", "-level", "40%,80%", "+level-colors", "white," + MINT, t("bubfill.png"))
    sh("-size", "1024x1024", "xc:#07090d", t("dotcol.png"))
    return t


def render(tmp, t, art_only):
    """Returns path of the 1024 composite. art_only = transparent canvas with just glow-less artwork."""
    base = t("bg.png")
    out = t("full.png" if not art_only else "art.png")
    if art_only:
        sh("-size", "1024x1024", "xc:#0DBD8B00", "-type", "TrueColorAlpha", out)
    else:
        sh(base, t("sq.mask.png"), "-compose", "copy_opacity", "-composite", out)
    # rim
    if not art_only:
        sh("-size", "1024x1024", "xc:white", t("rim.mask.png"), "-compose", "copy_opacity", "-composite", "-channel", "A", "-evaluate", "multiply", "0.12", "+channel", t("rimc.png"))
        sh(out, t("rimc.png"), "-compose", "over", "-composite", out)
    # bubble
    sh(t("bubfill.png"), t("bub.mask.png"), "-compose", "copy_opacity", "-composite", t("bubc.png"))
    sh(out, t("bubc.png"), "-compose", "over", "-composite", out)
    sh(t("dotcol.png"), t("dots.mask.png"), "-compose", "copy_opacity", "-composite", "-channel", "A", "-evaluate", "multiply", "0.82", "+channel", t("dotc.png"))
    sh(out, t("dotc.png"), "-compose", "over", "-composite", out)
    return out


def main():
    tmp = tempfile.mkdtemp()
    try:
        t = build(tmp)
        full = render(tmp, t, False)
        art = render(tmp, t, True)
        # halo for the foreground: faint glow only inside the safe area
        icon = os.path.join(ROOT, "assets/icon")
        open(os.path.join(icon, "superchat.svg"), "w").write(master_svg())
        for n in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
            sh(full, "-filter", "Lanczos", "-resize", f"{n}x{n}", "-depth", "8", os.path.join(icon, f"superchat-{n}.png"))
        sh(*[os.path.join(icon, f"superchat-{n}.png") for n in (16, 24, 32, 48, 64, 128, 256)], os.path.join(icon, "superchat.ico"))
        # Android adaptive icon: 432px canvas, artwork inside the 66% safe zone, background colour = the dark base
        res = os.path.join(ROOT, "apps/android/app/src/main/res")
        sh(art, "-filter", "Lanczos", "-resize", "460x460", "-background", "none", "-gravity", "center", "-extent", "432x432", "-depth", "8",
           os.path.join(res, "drawable-nodpi/ic_launcher_fg.png"))
        # admin favicon + logo
        web = os.path.join(ROOT, "admin-web/public")
        sh(full, "-filter", "Lanczos", "-resize", "64x64", "-depth", "8", os.path.join(web, "favicon.png"))
        sh(full, "-filter", "Lanczos", "-resize", "128x128", "-depth", "8", os.path.join(web, "logo.png"))
        # slint window/tray icon
        slint = os.path.join(ROOT, "apps/desktop/ui/assets/icon.png")
        if os.path.exists(slint):
            sh(full, "-filter", "Lanczos", "-resize", "256x256", "-depth", "8", slint)
        # previews of masks for checking (not committed): kept in /tmp
        shutil.copy(full, "/tmp/claude-1000/ic/full.png")
    finally:
        shutil.rmtree(tmp)


if __name__ == "__main__":
    main()
