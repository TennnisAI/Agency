"""Turns recorder stills into the site's screenshots.

Each gets rounded corners in its alpha, so the site's drop-shadow filter traces
the window rather than a rectangle, then is downscaled and written as WebP.
Needs Pillow and cwebp.
"""
import os
import subprocess
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
STILLS = os.path.join(HERE, "..", "out", "stills")
IMG = os.path.join(HERE, "..", "..", "..", "site", "assets", "img")

# still -> (site image, width). The overview is also the repo README's image.
SHOTS = {
    "issue-detail": ("shot-issues", 2400),
    "docs": ("shot-docs", 2400),
    "grid": ("shot-grid", 2400),
    "review-diff": ("shot-merge", 2400),
    "overview": ("shot-overview", 2560),
}
RADIUS = 24  # at the 2x capture scale, a macOS window's corner

for still, (name, width) in SHOTS.items():
    im = Image.open(os.path.join(STILLS, still + ".png")).convert("RGBA")
    w, h = im.size
    # Draw the mask at 4x and scale it down, for an antialiased edge.
    mask = Image.new("L", (w * 4, h * 4), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, w * 4 - 1, h * 4 - 1), radius=RADIUS * 4, fill=255)
    im.putalpha(mask.resize((w, h), Image.LANCZOS))
    im = im.resize((width, round(h * width / w)), Image.LANCZOS)
    png = os.path.join(STILLS, name + ".png")
    im.save(png)
    subprocess.run(["cwebp", "-quiet", "-q", "82", "-alpha_q", "100", "-m", "6", png,
                    "-o", os.path.join(IMG, name + ".webp")], check=True)
    print(name, im.size)
