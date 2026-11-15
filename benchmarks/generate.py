#!/usr/bin/env python3
"""Generate the NOVA benchmark templates.

    python3 benchmarks/generate.py            # writes benchmarks/*.json
    scripts/make-bench-assets.sh              # creates benchmarks/assets/ (once)

Everything is deterministic (fixed seed), so the same workloads can be
re-run after any engine change and on any machine.

Workloads (1920x1080, 30 fps):
  mixed-4min    7200 frames  the reference job: a 4-minute video of 30 scenes
  text-heavy     900 frames  36 animated text layers
  image-heavy    900 frames  14 image layers, always moving, with shadows
  shapes         900 frames  220 animated vector shapes
  static-spans   900 frames  long holds with short animated moments
"""
import json
import math
import os
import random

W, H, FPS = 1920, 1080, 30
HERE = os.path.dirname(os.path.abspath(__file__))

FONTS = [
    {"id": "regular", "type": "font", "src": "assets/NotoSans-Regular.ttf", "family": "Noto Sans"},
    {"id": "bold", "type": "font", "src": "assets/NotoSans-Bold.ttf", "family": "Noto Sans"},
]
IMAGES = [
    {"id": f"photo{i}", "type": "image", "src": f"assets/photo{i}.jpg"} for i in (1, 2, 3, 4)
] + [{"id": "detail", "type": "image", "src": "assets/detail1.jpg"}]

PALETTES = [
    ("#141a3a", "#0b1020", "#ffd166", "#ef476f"),
    ("#0f2027", "#2c5364", "#80ed99", "#48dbfb"),
    ("#2b1055", "#7597de", "#feca57", "#ff9ff3"),
    ("#1f1c2c", "#928dab", "#ff6b6b", "#1dd1a1"),
    ("#0b132b", "#3a506b", "#5bc0be", "#f9c74f"),
]
WORDS = ("Laju Tepat Kemas Ringan Jelas Tajam Pantas Stabil Mudah Padu Kukuh Segar "
         "render video templat animasi teks gambar warna gerak lancar kualiti masa kos").split()


def comp(cid, frames, layers, assets):
    return {
        "schema_version": "1.0",
        "composition": {"id": cid, "width": W, "height": H, "fps": FPS, "duration_frames": frames, "background": "#0b1020"},
        "assets": assets,
        "layers": layers,
    }


def kf(prop, *keys):
    """keys: (frame, value) or (frame, value, easing)"""
    out = []
    for k in keys:
        item = {"frame": k[0], "value": k[1]}
        if len(k) > 2:
            item["easing"] = k[2]
        out.append(item)
    return {"property": prop, "keyframes": out}


def text(lid, value, x, y, size, color="#ffffff", bold=False, anchor=(0, 0), **extra):
    t = {"value": value, "font_family": "Noto Sans", "font_size": size, "color": color}
    if bold:
        t["font_weight"] = 700
    for key in ("max_width", "align", "line_height", "letter_spacing"):
        if key in extra:
            t[key] = extra.pop(key)
    layer = {"id": lid, "type": "text", "text": t,
             "transform": {"position": {"x": x, "y": y}, "anchor": {"x": anchor[0], "y": anchor[1]}}}
    layer.update(extra)
    return layer


def backdrop(lid, c0, c1):
    return {"id": lid, "type": "rect", "size": {"width": W, "height": H},
            "fill": {"type": "linear", "from": [0, 0], "to": [1, 1],
                     "stops": [{"offset": 0, "color": c0}, {"offset": 1, "color": c1}]}}


def fade_in_out(length, fade=10):
    return kf("opacity", (0, 0), (fade, 1), (length - fade, 1), (length, 0))


def rise(y, start, dur=18, dist=70):
    return [kf("transform.position.y", (start, y + dist, "ease_out_cubic"), (start + dur, y)),
            kf("opacity", (start, 0), (start + dur - 4, 1))]


def photo_card(p, asset, x, y, size, length, slide_from, start=4):
    """Rounded photo with a drop shadow, sliding in, then slowly zooming."""
    return {
        "id": f"{p}-card", "type": "group", "size": {"width": size, "height": size},
        "transform": {"position": {"x": x, "y": y}, "anchor": {"x": 0.5, "y": 0.5}},
        "animations": [kf("transform.position.x", (start, slide_from, {"spring": {"stiffness": 70, "damping": 14}}), (start + 44, x))],
        "layers": [
            {"id": f"{p}-shadow", "type": "rect", "size": {"width": size, "height": size}, "corner_radius": 40,
             "fill": "#0b1020", "shadow": {"y": 26, "blur": 70, "color": "rgba(0,0,0,0.55)"}},
            {"id": f"{p}-frame", "type": "group", "size": {"width": size, "height": size}, "clip": True, "corner_radius": 40,
             "layers": [{"id": f"{p}-img", "type": "image", "asset": asset, "size": {"width": size, "height": size}, "fit": "cover",
                         "transform": {"position": {"x": size / 2, "y": size / 2}, "anchor": {"x": 0.5, "y": 0.5}},
                         "animations": [kf("transform.scale", (0, 1.0), (length, 1.12))]}]},
        ],
    }


def scene_hero(p, n, length, pal, rng):
    c0, c1, accent, pop = pal
    return [
        backdrop(f"{p}-bg", c0, c1),
        photo_card(p, f"photo{n % 4 + 1}", 1400, 520, 720, length, 2500),
        text(f"{p}-kicker", f"BAHAGIAN {n + 1:02d}", 130, 250, 34, accent, True, letter_spacing=6, animations=rise(250, 6, 14, 30)),
        text(f"{p}-title", "Render lebih laju,\nkualiti kekal tajam", 130, 310, 92, "#ffffff", True, line_height=1.08,
             animations=rise(310, 10)),
        {"id": f"{p}-rule", "type": "path", "d": "M0 0 C140 -24 330 22 560 -4",
         "stroke": {"color": accent, "width": 10, "cap": "round", "trim_end": 0},
         "transform": {"position": {"x": 136, "y": 560}},
         "animations": [kf("stroke.trim_end", (30, 0, "ease_in_out_cubic"), (56, 1))]},
        text(f"{p}-body", "Templat JSON masuk, video MP4 keluar. Tanpa pelayar, tanpa GPU.", 130, 610, 40, "#c9d1ff",
             max_width=820, line_height=1.35, animations=rise(610, 38)),
        {"id": f"{p}-badge", "type": "group", "size": {"width": 300, "height": 100},
         "transform": {"position": {"x": 280, "y": 860}, "anchor": {"x": 0.5, "y": 0.5}},
         "animations": [kf("transform.scale", (64, 0, "spring"), (94, 1))],
         "layers": [{"id": f"{p}-badge-bg", "type": "rect", "size": {"width": 300, "height": 100}, "corner_radius": 50, "fill": pop},
                    text(f"{p}-badge-t", "BARU", 150, 50, 46, "#14182b", True, (0.5, 0.5))]},
    ]


def scene_quote(p, n, length, pal, rng):
    c0, c1, accent, pop = pal
    rings = []
    for i in range(3):
        d = 380 + i * 170
        rings.append({"id": f"{p}-ring{i}", "type": "ellipse", "size": {"width": d, "height": d},
                      "stroke": {"color": accent if i % 2 else pop, "width": 6, "trim_start": 0.0, "trim_end": 0.72, "cap": "round"},
                      "opacity": 0.5,
                      "transform": {"position": {"x": 1560, "y": 540}, "anchor": {"x": 0.5, "y": 0.5}},
                      "animations": [kf("transform.rotation", (0, i * 40), (length, i * 40 + (90 if i % 2 else -90)))]})
    return [
        backdrop(f"{p}-bg", c1, c0),
        *rings,
        text(f"{p}-mark", "“", 150, 120, 300, accent, True, animations=[kf("opacity", (4, 0), (18, 1))]),
        text(f"{p}-quote", "Masa render bukan lagi sebab untuk menangguhkan video seterusnya.", 150, 360, 76, "#ffffff", True,
             max_width=1150, line_height=1.18, animations=rise(360, 12, 20)),
        text(f"{p}-who", "— Pasukan Produksi", 150, 760, 40, "#c9d1ff", animations=rise(760, 44, 16, 40)),
    ]


def scene_split(p, n, length, pal, rng):
    c0, c1, accent, pop = pal
    out = [backdrop(f"{p}-bg", c0, c1)]
    for i, x in enumerate((520, 1400)):
        out.append(photo_card(f"{p}-{i}", f"photo{(n + i) % 4 + 1}" if i else "detail", x, 470, 620, length,
                              -700 if i == 0 else 2700, 4 + i * 8))
        out.append(text(f"{p}-cap{i}", ("Sebelum" if i == 0 else "Selepas"), x, 850, 48, "#ffffff", True, (0.5, 0.5),
                        animations=rise(850, 40 + i * 8, 14, 30)))
    out.append({"id": f"{p}-bar", "type": "rect", "size": {"width": 4, "height": 14}, "corner_radius": 7, "fill": accent,
                "transform": {"position": {"x": 960, "y": 960}, "anchor": {"x": 0.5, "y": 0.5}},
                "animations": [kf("size.width", (50, 4, "ease_out_cubic"), (80, 900))]})
    return out


def scene_stats(p, n, length, pal, rng):
    c0, c1, accent, pop = pal
    out = [backdrop(f"{p}-bg", c1, c0),
           text(f"{p}-title", "Angka yang penting", 130, 110, 72, "#ffffff", True, animations=rise(110, 4))]
    values = [0.45, 0.8, 0.62, 0.95]
    labels = ["Q1", "Q2", "Q3", "Q4"]
    base_y, max_h = 880, 520
    for i, v in enumerate(values):
        x = 260 + i * 280
        hgt = max_h * v
        out.append({"id": f"{p}-col{i}", "type": "rect", "size": {"width": 170, "height": 2}, "corner_radius": 16,
                    "fill": accent if i % 2 == 0 else pop,
                    "transform": {"position": {"x": x, "y": base_y}, "anchor": {"x": 0, "y": 1}},
                    "animations": [kf("size.height", (16 + i * 6, 2, {"spring": {"stiffness": 90, "damping": 12}}), (56 + i * 6, hgt))]})
        out.append(text(f"{p}-lab{i}", labels[i], x + 85, base_y + 50, 40, "#c9d1ff", False, (0.5, 0.5)))
        out.append(text(f"{p}-val{i}", f"{int(v * 100)}%", x + 85, base_y - hgt - 44, 46, "#ffffff", True, (0.5, 0.5),
                        animations=[kf("opacity", (50 + i * 6, 0), (62 + i * 6, 1))]))
    out.append({"id": f"{p}-trend", "type": "path", "d": "M0 220 L140 90 L280 150 L420 20", "opacity": 0.9,
                "stroke": {"color": "#ffffff", "width": 8, "cap": "round", "join": "round", "trim_end": 0},
                "transform": {"position": {"x": 1390, "y": 420}},
                "animations": [kf("stroke.trim_end", (70, 0, "ease_in_out_cubic"), (120, 1))]})
    out.append(text(f"{p}-note", "Naik setiap suku tahun", 1600, 720, 40, "#c9d1ff", False, (0.5, 0.5),
                    animations=[kf("opacity", (110, 0), (126, 1))]))
    return out


def scene_blobs(p, n, length, pal, rng):
    """The expensive kind: large blurred shapes that never stop moving."""
    c0, c1, accent, pop = pal
    blobs = []
    for i, col in enumerate((accent, pop, "#7b2ff7")):
        x0, y0 = rng.randint(300, 1600), rng.randint(200, 880)
        x1, y1 = rng.randint(300, 1600), rng.randint(200, 880)
        d = rng.randint(520, 760)
        blobs.append({"id": f"{p}-blob{i}", "type": "ellipse", "size": {"width": d, "height": d}, "fill": col, "opacity": 0.55,
                      "blur": 70, "transform": {"position": {"x": x0, "y": y0}, "anchor": {"x": 0.5, "y": 0.5}},
                      "animations": [kf("transform.position.x", (0, x0, "ease_in_out_cubic"), (length, x1)),
                                     kf("transform.position.y", (0, y0, "ease_in_out_cubic"), (length, y1))]})
    return [
        {"id": f"{p}-bg", "type": "solid", "color": c0},
        *blobs,
        text(f"{p}-title", "Latar bergerak", 960, 470, 130, "#ffffff", True, (0.5, 0.5),
             shadow={"y": 8, "blur": 30, "color": "rgba(0,0,0,0.5)"},
             animations=[kf("transform.scale", (6, 0.8, "spring"), (40, 1)), kf("opacity", (6, 0), (20, 1))]),
        text(f"{p}-sub", "kabur Gaussian besar pada setiap frame", 960, 620, 44, "#e6e9ff", False, (0.5, 0.5),
             animations=rise(620, 30, 16, 30)),
    ]


SCENES = [scene_hero, scene_quote, scene_split, scene_stats, scene_blobs]


def mixed(scene_count=30, scene_len=240, progress_bar=True):
    rng = random.Random(7)
    total = scene_count * scene_len
    layers = []
    for n in range(scene_count):
        p = f"s{n:02d}"
        make = SCENES[n % len(SCENES)]
        layers.append({"id": p, "type": "group", "start_frame": n * scene_len, "end_frame": (n + 1) * scene_len,
                       "animations": [fade_in_out(scene_len)],
                       "layers": make(p, n, scene_len, PALETTES[n % len(PALETTES)], rng)})
    # Captions: a new page every 24 frames for the whole video.
    page = 24
    for i in range(total // page):
        words = " ".join(rng.choice(WORDS) for _ in range(rng.randint(2, 4)))
        layers.append(text(f"cap{i:03d}", words, 960, 1000, 46, "#ffffff", True, (0.5, 0.5),
                           start_frame=i * page, end_frame=(i + 1) * page,
                           shadow={"y": 3, "blur": 12, "color": "rgba(0,0,0,0.85)"}))
    if progress_bar:
        # Changes on every frame of the video, so no two frames are identical:
        # the result does not lean on whole-frame reuse.
        layers.append({"id": "progress", "type": "rect", "size": {"width": 2, "height": 8}, "fill": "#ffd166",
                       "transform": {"position": {"x": 0, "y": H - 8}},
                       "animations": [kf("size.width", (0, 2), (total, W))]})
    return comp("mixed-4min", total, layers, FONTS + IMAGES)


def text_heavy(frames=900):
    rng = random.Random(11)
    layers = [backdrop("bg", "#141a3a", "#0b1020")]
    for i in range(36):
        col, row = i % 4, i // 4
        x, y = 80 + col * 460, 60 + row * 112
        size = rng.choice((28, 34, 42, 54))
        value = " ".join(rng.choice(WORDS) for _ in range(rng.randint(2, 4)))
        phase = rng.randint(0, 60)
        layers.append(text(f"t{i:02d}", value, x, y, size, rng.choice(("#ffffff", "#ffd166", "#c9d1ff", "#80ed99")), i % 3 == 0,
                           letter_spacing=rng.choice((0, 0, 2)),
                           animations=[kf("transform.position.x", (0, x - 30), (frames / 2 + phase, x + 30, "ease_in_out_cubic"), (frames, x - 30)),
                                       kf("opacity", (0, 0.35), (phase + 30, 1), (frames - 30, 1), (frames, 0.35))]))
    return comp("text-heavy", frames, layers, FONTS)


def image_heavy(frames=900):
    rng = random.Random(13)
    layers = [backdrop("bg", "#0f2027", "#2c5364")]
    ids = ["photo1", "photo2", "photo3", "photo4", "detail"]
    for i in range(14):
        size = rng.choice((260, 320, 380))
        x0, y0 = rng.randint(200, 1700), rng.randint(180, 900)
        x1, y1 = rng.randint(200, 1700), rng.randint(180, 900)
        layers.append({"id": f"im{i:02d}", "type": "image", "asset": ids[i % 5], "size": {"width": size, "height": size * 0.75},
                       "fit": "cover", "shadow": {"y": 12, "blur": 30, "color": "rgba(0,0,0,0.5)"},
                       "transform": {"position": {"x": x0, "y": y0}, "anchor": {"x": 0.5, "y": 0.5}},
                       "animations": [kf("transform.position.x", (0, x0, "ease_in_out_cubic"), (frames, x1)),
                                      kf("transform.position.y", (0, y0, "ease_in_out_cubic"), (frames, y1)),
                                      kf("transform.rotation", (0, rng.randint(-12, 12)), (frames, rng.randint(-12, 12))),
                                      kf("transform.scale", (0, 0.9), (frames, 1.2))]})
    return comp("image-heavy", frames, layers, IMAGES)


def shapes(frames=900):
    rng = random.Random(17)
    layers = [{"id": "bg", "type": "solid", "color": "#0b132b"}]
    star = "M50 0 L61 35 L98 35 L68 57 L79 91 L50 70 L21 91 L32 57 L2 35 L39 35 Z"
    for i in range(220):
        x0, y0 = rng.randint(0, W), rng.randint(0, H)
        x1, y1 = rng.randint(0, W), rng.randint(0, H)
        col = rng.choice(("#ffd166", "#ef476f", "#06d6a0", "#118ab2", "#f78c6b", "#ffffff"))
        kind = i % 4
        base = {"id": f"sh{i:03d}", "opacity": rng.choice((0.6, 0.8, 1.0)),
                "transform": {"position": {"x": x0, "y": y0}, "anchor": {"x": 0.5, "y": 0.5}},
                "animations": [kf("transform.position.x", (0, x0), (frames, x1)),
                               kf("transform.position.y", (0, y0), (frames, y1)),
                               kf("transform.rotation", (0, 0), (frames, rng.choice((-360, 360, 180))))]}
        d = rng.randint(30, 120)
        if kind == 0:
            base.update(type="rect", size={"width": d, "height": d}, corner_radius=d * 0.2, fill=col)
        elif kind == 1:
            base.update(type="ellipse", size={"width": d, "height": d * 0.7}, fill=col)
        elif kind == 2:
            base.update(type="path", d=star, size={"width": 100, "height": 91}, fill=col)
        else:
            base.update(type="ellipse", size={"width": d, "height": d}, stroke={"color": col, "width": 6})
        layers.append(base)
    return comp("shapes", frames, layers, [])


def static_spans(frames=900):
    layers = [backdrop("bg", "#2b1055", "#7597de"),
              {"id": "panel", "type": "rect", "size": {"width": 1500, "height": 700}, "corner_radius": 40, "fill": "rgba(11,16,32,0.85)",
               "shadow": {"y": 20, "blur": 60, "color": "rgba(0,0,0,0.5)"}, "transform": {"position": {"x": 210, "y": 190}}},
              text("brand", "NOVA", 290, 250, 120, "#ffffff", True, letter_spacing=16)]
    # a new line of text every 150 frames; each animates for 20 frames, then everything holds
    for i in range(frames // 150):
        s = i * 150
        layers.append(text(f"line{i}", f"Perkara {i + 1}: semuanya pegun sehingga baris seterusnya", 290, 450 + i * 64, 44, "#c9d1ff",
                           start_frame=s, animations=rise(450 + i * 64, 0, 20, 40)))
    return comp("static-spans", frames, layers, FONTS)


def main():
    jobs = {"mixed-4min": mixed(), "text-heavy": text_heavy(), "image-heavy": image_heavy(),
            "shapes": shapes(), "static-spans": static_spans()}
    for name, template in jobs.items():
        path = os.path.join(HERE, f"{name}.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(template, f, ensure_ascii=False, separators=(",", ":"))
        c = template["composition"]
        print(f"{name:<13} {c['duration_frames']:>5} frames  {os.path.getsize(path) / 1024:7.1f} KiB  {path}")


if __name__ == "__main__":
    main()
