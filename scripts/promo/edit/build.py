"""Cuts the recorded clips into the two masters, at 2560x1440 and 2240x1400.

tour-master.mp4: intro card, five captioned scenes in a rounded window, end card.
hero-master.mp4: a silent loop of the raw UI for the site's hero window.

Needs out/clips from ../run.sh and out/edit from render-cards.mjs.
"""
import os
import subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "out")
CLIPS = os.path.join(OUT, "clips")
E = os.path.join(OUT, "edit")
SEG = os.path.join(E, "seg")

# The window sits at (X, Y) on the 2560x1440 canvas: cards.html draws its
# shadow, mask and border at the same rectangle.
X, Y, W, H = 320, 184, 1920, 1200

# Per scene: (start, end, speed) spans of the source clip, in seconds. The
# fast spans are typing into dialogs, which reads fine at twice the speed.
TOUR = {
    "overview": [(0.3, 6.8, 1)],
    "issues": [(0.3, 10.8, 1)],
    "race": [(1.0, 3.2, 1), (3.2, 10.2, 2.2), (10.2, 17.6, 1)],
    "loop": [(1.0, 11.6, 2.5), (11.6, 30.7, 1.8)],
    "review": [(0.5, 11.5, 1.6), (11.5, 19.5, 1)],
}
XFADE = 0.5


def ff(args):
    subprocess.run(["ffmpeg", "-v", "error", "-y"] + args, check=True)


def duration(path):
    return float(subprocess.check_output(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path]))


def crossfade(inputs, out, t, extra_out=None):
    """Joins inputs with fades of t seconds; returns the joined length."""
    durs = [duration(p) for p in inputs]
    f, prev, off = [], "[0:v]", 0.0
    for i in range(1, len(inputs)):
        off += durs[i - 1] - t
        f.append(f"{prev}[{i}:v]xfade=transition=fade:duration={t}:offset={off:.3f}[x{i}]")
        prev = f"[x{i}]"
    args = []
    for p in inputs:
        args += ["-i", p]
    ff(args + ["-filter_complex", ";".join(f), "-map", prev] + (extra_out or []) + [out])
    return off + durs[-1]


def tour():
    os.makedirs(SEG, exist_ok=True)
    for name in ("intro", "end"):
        ff(["-r", "30", "-i", os.path.join(E, f"frames-{name}", "%04d.png"),
            "-c:v", "libx264", "-crf", "12", "-pix_fmt", "yuv420p", os.path.join(SEG, f"{name}.mp4")])
    segs = [os.path.join(SEG, "intro.mp4")]
    for k, spans in TOUR.items():
        dur = f"{sum((b - a) / s for a, b, s in spans):.3f}"
        f = [f"[0:v]trim={a}:{b},setpts=(PTS-STARTPTS)/{s}[p{j}]" for j, (a, b, s) in enumerate(spans)]
        f.append("".join(f"[p{j}]" for j in range(len(spans)))
                 + f"concat=n={len(spans)}:v=1:a=0,fps=30,scale={W}:{H}:flags=lanczos,format=rgba[v]")
        f.append(f"[2:v]crop={W}:{H}:{X}:{Y},format=gray[m]")
        f.append("[v][m]alphamerge[vm]")
        f.append(f"[1:v][vm]overlay={X}:{Y}:shortest=1[c]")
        f.append("[c][3:v]overlay=0:0:shortest=1,format=yuv420p[out]")
        # Every looped still gets the scene's exact length: an unbounded
        # -loop 1 input kept the filter graph alive for an hour after the
        # clip had ended.
        still = lambda p: ["-loop", "1", "-t", dur, "-i", os.path.join(E, p)]
        seg = os.path.join(SEG, f"{k}.mp4")
        ff(["-i", os.path.join(CLIPS, f"{k}.mp4")] + still(f"bg-{k}.png") + still("mask.png") + still("border.png")
           + ["-filter_complex", ";".join(f), "-map", "[out]", "-t", dur, "-r", "30",
              "-c:v", "libx264", "-crf", "12", "-preset", "medium", seg])
        segs.append(seg)
    segs.append(os.path.join(SEG, "end.mp4"))
    total = crossfade(segs, os.path.join(OUT, "tour-master.mp4"), XFADE,
                      ["-c:v", "libx264", "-crf", "16", "-preset", "slow", "-pix_fmt", "yuv420p", "-movflags", "+faststart"])
    print(f"tour-master.mp4 {total:.1f}s")


def hero():
    # A B C D A2, crossfaded, then the first second dropped, so the last frame
    # (A at 1.0s) runs straight into the first (A just after 1.0s) and the
    # loop has no seam.
    w, h, t = 2240, 1400, 0.6
    parts = [("overview", 0.0, 5.6), ("grid", 2.4, 8.6), ("race", 10.4, 17.0), ("review", 13.8, 19.5), ("overview", 0.0, 1.0 + t)]
    args, f = [], []
    for i, (k, a, b) in enumerate(parts):
        args += ["-i", os.path.join(CLIPS, f"{k}.mp4")]
        f.append(f"[{i}:v]trim={a}:{b},setpts=PTS-STARTPTS,fps=30,scale={w}:{h}:flags=lanczos,format=yuv420p[s{i}]")
    prev, off = "[s0]", 0.0
    for i in range(1, len(parts)):
        off += (parts[i - 1][2] - parts[i - 1][1]) - t
        f.append(f"{prev}[s{i}]xfade=transition=fade:duration={t}:offset={off:.3f}[x{i}]")
        prev = f"[x{i}]"
    total = off + (parts[-1][2] - parts[-1][1])
    f.append(f"{prev}trim=1.0:{total:.3f},setpts=PTS-STARTPTS[out]")
    ff(args + ["-filter_complex", ";".join(f), "-map", "[out]", "-c:v", "libx264", "-crf", "12",
               "-preset", "medium", "-pix_fmt", "yuv420p", os.path.join(OUT, "hero-master.mp4")])
    print(f"hero-master.mp4 {total - 1.0:.1f}s")


if __name__ == "__main__":
    tour()
    hero()
