#!/usr/bin/env python3
"""KES #01 — "Jam Emas Dalam Gelap", rebuilt as a NOVA template (no voice).

    python3 examples/kes01/generate.py
    nova render examples/kes01/template.json -o out/kes01.mp4

A port of the Remotion episode in ~/Documents/motion (src/episodes/Kes01.tsx):
same story, timing (the silent `dur` values), layout, colours and fonts.
The script writes template.json and the SVG illustrations next to itself.

How the port works: every animated value of the original is a function of
the scene's frame. Those functions are evaluated here, frame by frame, and
written as keyframes (dropping the ones that lie on a straight line), so
motion matches the original at every frame without the engine needing to
know springs-with-offsets, sine wobbles or wrap-around.
"""
import json
import math
import os
import random

W, H, FPS = 1080, 1920, 30
HERE = os.path.dirname(os.path.abspath(__file__))
SVG_DIR = os.path.join(HERE, "assets", "svg")

C = dict(bg="#0d1118", bg2="#151b27", bg3="#1d2535", paper="#efe8da", paperDark="#d8cdb6", ink="#1a1c23",
         inkSoft="#4a4d57", accent="#ff5a4e", gold="#f2c14e", insight="#7ee0ff", white="#f7f4ee", grey="#8a90a0",
         lampOn="#ffd27a", lampOff="#1c2333")
CAST = dict(ikmal=("IKMAL", "#ff5a4e"), aina=("AINA", "#ff9ecb"), makcik=("MAK CIK ROSNAH", "#c08bff"),
            zul=("ENCIK ZUL", "#f5b942"), lina=("KAK LINA", "#4fd1c5"), daniel=("DANIEL", "#6aa8ff"))
DISPLAY, MONO, SANS = "Bebas Neue", "Special Elite", "Plus Jakarta Sans"

rng = random.Random(1)
ASSETS = {}
_uid = [0]


def uid(prefix):
    _uid[0] += 1
    return f"{prefix}-{_uid[0]}"


# ------------------------------------------------------------------ maths
def spring(frames, damping, stiffness):
    """Remotion spring(): 0 -> 1, mass 1, at `frames` after its start."""
    if frames <= 0:
        return 0.0
    t = frames / FPS
    w0 = math.sqrt(stiffness)
    zeta = damping / (2 * math.sqrt(stiffness))
    if zeta < 1:
        wd = w0 * math.sqrt(1 - zeta * zeta)
        return 1 - math.exp(-zeta * w0 * t) * (math.cos(wd * t) + (zeta * w0 / wd) * math.sin(wd * t))
    return 1 - math.exp(-w0 * t) * (1 + w0 * t)


def interp(x, xs, ys, ease=None):
    """Remotion interpolate() with clamping on both sides."""
    if x <= xs[0]:
        return ys[0]
    if x >= xs[-1]:
        return ys[-1]
    for i in range(len(xs) - 1):
        if xs[i] <= x <= xs[i + 1]:
            u = (x - xs[i]) / (xs[i + 1] - xs[i])
            if ease:
                u = ease(u)
            return ys[i] + (ys[i + 1] - ys[i]) * u
    return ys[-1]


def out_cubic(u):
    return 1 - (1 - u) ** 3


def in_out_cubic(u):
    return 4 * u ** 3 if u < 0.5 else 1 - (-2 * u + 2) ** 3 / 2


def rgba(hex_color, alpha):
    h = hex_color.lstrip("#")
    return f"rgba({int(h[0:2], 16)},{int(h[2:4], 16)},{int(h[4:6], 16)},{alpha})"


# ------------------------------------------------------------------ keyframes
def sampled(fn, start, end, step=1, eps=0.02):
    """Evaluate fn(scene_frame) over [start, end) and return keyframes in the
    layer's local time, keeping only points that are not on a straight line."""
    frames = list(range(start, end, step))
    if frames[-1] != end - 1:
        frames.append(end - 1)
    pts = [(f - start, fn(f)) for f in frames]
    keep = [pts[0]]
    anchor = 0
    for i in range(2, len(pts)):
        (x0, y0), (x1, y1) = pts[anchor], pts[i]
        ok = True
        for j in range(anchor + 1, i):
            xj, yj = pts[j]
            if abs(y0 + (y1 - y0) * (xj - x0) / (x1 - x0) - yj) > eps:
                ok = False
                break
        if not ok:
            keep.append(pts[i - 1])
            anchor = i - 1
    if len(pts) > 1:
        keep.append(pts[-1])
    return [{"frame": x, "value": round(y, 4)} for x, y in keep]


STATIC = {
    "opacity": lambda l, v: l.__setitem__("opacity", v),
    "transform.position.x": lambda l, v: l["transform"]["position"].__setitem__("x", v),
    "transform.position.y": lambda l, v: l["transform"]["position"].__setitem__("y", v),
    "transform.rotation": lambda l, v: l["transform"].__setitem__("rotation", v),
    "transform.scale": lambda l, v: l["transform"].__setitem__("scale", v),
    "transform.scale.x": lambda l, v: l["transform"].__setitem__("scale", {"x": v, "y": 1}),
    "stroke.trim_end": lambda l, v: l["stroke"].__setitem__("trim_end", v),
    "text.reveal": lambda l, v: l["text"]["reveal"].__setitem__("count", v),
}


def animate(layer, prop, fn, step=1, eps=0.02):
    """Attach fn(scene_frame) to a property over the layer's lifetime."""
    start, end = layer["_range"]
    keys = sampled(fn, start, end, step, eps)
    values = {k["value"] for k in keys}
    if len(values) == 1:
        v = keys[0]["value"]
        if prop == "opacity":
            v = min(1.0, max(0.0, v))
        STATIC[prop](layer, v)
    else:
        if prop == "opacity":
            for k in keys:
                k["value"] = min(1.0, max(0.0, k["value"]))
        layer.setdefault("animations", []).append({"property": prop, "keyframes": keys})
    return layer


# ------------------------------------------------------------------ layers
SCENE = {"dur": 0}  # length of the scene currently being built


def L(kind, lid=None, x=0.0, y=0.0, anchor=(0, 0), start=None, end=None, **kw):
    """A layer. `start`/`end` are always *scene* frames, however deeply the
    layer is nested; finalize() turns them into parent-relative frames."""
    layer = {"id": uid(lid or kind), "type": kind,
             "transform": {"position": {"x": x, "y": y}, "anchor": {"x": anchor[0], "y": anchor[1]}}}
    layer["_timed"] = start is not None or end is not None
    layer["_range"] = (start or 0, SCENE["dur"] if end is None else end)
    layer.update(kw)
    return layer


def finalize(layer, parent_start=0):
    """Scene frames -> the engine's parent-relative start/end, recursively."""
    start, end = layer.pop("_range")
    timed = layer.pop("_timed")
    layer.pop("_dur", None)
    if timed:
        layer["start_frame"], layer["end_frame"] = start - parent_start, end - parent_start
        own = start
    else:
        # untimed layers live in their parent's clock
        assert not (layer.get("animations") and parent_start != 0), f"{layer['id']}: animated but untimed under a shifted parent"
        own = parent_start
    for child in layer.get("layers", []):
        finalize(child, own)
    return layer


def rect(w, h, fill, radius=0, **kw):
    extra = {"corner_radius": radius} if radius else {}
    return L("rect", size={"width": w, "height": h}, fill=fill, **extra, **kw)


def ellipse(w, h, fill=None, stroke=None, **kw):
    extra = {}
    if fill:
        extra["fill"] = fill
    if stroke:
        extra["stroke"] = stroke
    return L("ellipse", size={"width": w, "height": h}, **extra, **kw)


def path(d, fill=None, stroke=None, **kw):
    extra = {}
    if fill:
        extra["fill"] = fill
    if stroke:
        extra["stroke"] = stroke
    return L("path", d=d, **extra, **kw)


def group(layers, size=None, clip=False, radius=0, **kw):
    extra = {}
    if size:
        extra["size"] = {"width": size[0], "height": size[1]}
    if clip:
        extra["clip"] = True
        if radius:
            extra["corner_radius"] = radius
    return L("group", layers=[x for x in layers if x is not None], **extra, **kw)


def text(value, font, size, color, weight=None, spacing=0, line_height=None, align=None, max_width=None, bg=None,
         spans=None, reveal=None, hug=False, **kw):
    t = {"value": value, "font_family": font, "font_size": size, "color": color}
    if weight:
        t["font_weight"] = weight
    if spacing:
        t["letter_spacing"] = spacing
    if line_height:
        t["line_height"] = line_height
    if align:
        t["align"] = align
    if max_width:
        t["max_width"] = max_width
    if hug:
        t["hug"] = True
    if bg:
        t["background"] = bg
    if spans:
        t["spans"] = spans
    if reveal:
        t["reveal"] = reveal
    return L("text", text=t, **kw)


def panel(color=None, radius=0, pad=(0, 0, 0, 0), border=None):
    """Text background. `pad` is CSS order-free: (left, top, right, bottom)."""
    bw = border[1] if border else 0
    b = {"radius": radius, "padding": {"left": pad[0] + bw, "top": pad[1] + bw, "right": pad[2] + bw, "bottom": pad[3] + bw}}
    if color:
        b["color"] = color
    if border:
        b["border_color"], b["border_width"] = border
    return b


def svg(asset, w, h, **kw):
    return L("svg", asset=asset, size={"width": w, "height": h}, fit="fill", **kw)


def svg_asset(aid, content, width, height, viewbox=None):
    if aid not in ASSETS:
        vb = viewbox or f"0 0 {width} {height}"
        body = f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="{vb}">{content}</svg>\n'
        with open(os.path.join(SVG_DIR, f"{aid}.svg"), "w", encoding="utf-8") as f:
            f.write(body)
        ASSETS[aid] = {"id": aid, "type": "svg", "src": f"assets/svg/{aid}.svg"}
    return aid


# ------------------------------------------------------------------ characters
LOOKS = {
    "ikmal": dict(skin="#f1c7a0", shade="#d9a77f", hair="messy", hairColor="#1d1a1c", shirt="#2b3a55", shirtShade="#22304a", scarf=C["accent"]),
    "aina": dict(skin="#ebbd93", shade="#d29f74", hair="tudung", hairColor="#f08cb8", shirt="#f7d9e6", shirtShade="#e8c2d2"),
    "makcik": dict(skin="#d9a47c", shade="#bf885f", hair="tudung", hairColor="#7b3f8c", shirt="#5a2d66", shirtShade="#4a2354", glasses=True, wrinkles=True),
    "zul": dict(skin="#c98e64", shade="#ad7349", hair="short", hairColor="#2a2420", shirt="#c9893a", shirtShade="#ad722a", mustache=True),
    "lina": dict(skin="#e2ae86", shade="#c79068", hair="tudung", hairColor="#2f9c93", shirt="#1f6f69", shirtShade="#185a55"),
    "daniel": dict(skin="#f0c9a3", shade="#d6a882", hair="beanie", hairColor="#2c4a7c", shirt="#3a3f4b", shirtShade="#2e323c", strap=True),
}
INK = "#1b1a1f"
CHAR_H = 412  # the body reaches a little below the 400-unit square


def character_svg(cid, expr, mouth_open, blink):
    k = LOOKS[cid]
    tudung = k["hair"] == "tudung"
    face_cy = 198 if tudung else 190
    eye_y = 202 if tudung else 200
    mouth_y = 248 if tudung else 252
    o = []
    o.append(f'<path d="M40 410 C50 322 110 292 200 292 C290 292 350 322 360 410 Z" fill="{k["shirt"]}"/>')
    o.append(f'<path d="M40 410 C46 360 70 330 104 314 C96 350 100 380 110 410 Z" fill="{k["shirtShade"]}"/>')
    o.append(f'<path d="M172 246 L172 298 Q200 316 228 298 L228 246 Z" fill="{k["shade"]}"/>')
    if k.get("strap"):
        o.append('<path d="M118 306 L142 300 L312 410 L280 410 Z" fill="#17181c"/>'
                 '<rect x="238" y="352" width="74" height="50" rx="8" fill="#202228"/>'
                 '<circle cx="275" cy="377" r="17" fill="#3a3d46" stroke="#111" stroke-width="4"/>'
                 '<circle cx="275" cy="377" r="7" fill="#7ea6d8"/>')
    if tudung:
        hc = k["hairColor"]
        o.append(f'<path d="M200 84 C122 84 98 150 100 214 C102 262 112 288 76 326 C116 350 284 350 324 326 C288 288 298 262 300 214 C302 150 278 84 200 84 Z" fill="{hc}"/>'
                 f'<path d="M84 296 Q200 336 316 296 L342 380 Q200 420 58 380 Z" fill="{hc}"/>'
                 '<path d="M84 296 Q200 336 316 296" stroke="#000" stroke-opacity="0.18" stroke-width="6" fill="none"/>')
        o.append(f'<ellipse cx="200" cy="{face_cy}" rx="72" ry="82" fill="{k["skin"]}"/>'
                 f'<ellipse cx="200" cy="{face_cy}" rx="72" ry="82" fill="none" stroke="#000" stroke-opacity="0.15" stroke-width="5"/>')
    else:
        o.append(f'<circle cx="122" cy="204" r="16" fill="{k["skin"]}"/><circle cx="278" cy="204" r="16" fill="{k["skin"]}"/>')
        o.append(f'<ellipse cx="200" cy="{face_cy}" rx="80" ry="92" fill="{k["skin"]}"/>')
    if k["hair"] == "messy":
        o.append(f'<path d="M120 186 C108 120 140 84 200 82 C262 82 296 118 282 186 L272 152 L258 174 L248 140 L228 162 L214 128 L196 158 L178 128 L166 162 L148 140 L138 172 L128 150 Z M146 104 L132 60 L176 88 L194 46 L214 84 L254 52 L248 96 L290 88 L266 122 L150 122 Z" fill="{k["hairColor"]}"/>')
    if k["hair"] == "short":
        o.append(f'<path d="M120 178 C110 110 156 92 200 92 C246 92 292 110 280 178 C272 140 252 126 200 124 C148 126 128 140 120 178 Z" fill="{k["hairColor"]}"/>')
    if k["hair"] == "beanie":
        o.append('<path d="M122 188 C118 160 124 150 132 146 L132 190 Z M278 188 C282 160 276 150 268 146 L268 190 Z" fill="#3b2a22"/>'
                 f'<path d="M114 166 C114 92 160 68 200 68 C240 68 286 92 286 166 Z" fill="{k["hairColor"]}"/>'
                 '<rect x="108" y="146" width="184" height="36" rx="16" fill="#1f3760"/>'
                 '<path d="M140 150 L140 180 M170 150 L170 180 M200 150 L200 180 M230 150 L230 180 M260 150 L260 180" stroke="#2c4a7c" stroke-width="6"/>'
                 '<circle cx="200" cy="64" r="19" fill="#e8e1d2"/>')
    o.append(f'<circle cx="150" cy="{eye_y + 32}" r="13" fill="#ff7b7b" opacity="0.28"/><circle cx="250" cy="{eye_y + 32}" r="13" fill="#ff7b7b" opacity="0.28"/>')

    # brows
    l, r = [150, 172, 184, 172], [216, 172, 250, 172]
    if expr in ("determined", "angry"):
        l[1], l[3], r[1], r[3] = 164, 178, 178, 164
    elif expr == "shock":
        l[1], l[3], r[1], r[3] = 158, 154, 154, 158
    elif expr in ("nervous", "sad"):
        l[1], l[3], r[1], r[3] = 176, 164, 164, 176
    elif expr == "sly":
        l[1], l[3], r[1], r[3] = 174, 174, 160, 168
    brow_color = "#3a2a24" if tudung else k["hairColor"]
    for b in (l, r):
        o.append(f'<line x1="{b[0]}" y1="{b[1] + eye_y - 200}" x2="{b[2]}" y2="{b[3] + eye_y - 200}" stroke="{brow_color}" stroke-width="7" stroke-linecap="round"/>')

    # eyes
    def eye(cx):
        if expr == "shock":
            return f'<circle cx="{cx}" cy="{eye_y}" r="17" fill="#fff" stroke="{INK}" stroke-width="3"/><circle cx="{cx}" cy="{eye_y}" r="7" fill="{INK}"/>'
        if expr == "sleepy":
            return f'<path d="M{cx - 13} {eye_y} Q{cx} {eye_y + 9} {cx + 13} {eye_y}" stroke="{INK}" stroke-width="5" fill="none" stroke-linecap="round"/>'
        if expr == "sly":
            return (f'<ellipse cx="{cx}" cy="{eye_y + 3}" rx="10" ry="7" fill="{INK}"/>'
                    f'<path d="M{cx - 15} {eye_y - 2} L{cx + 15} {eye_y - 2}" stroke="{INK}" stroke-width="5" stroke-linecap="round"/>')
        return f'<ellipse cx="{cx}" cy="{eye_y}" rx="9" ry="11" fill="{INK}"/><circle cx="{cx + 3}" cy="{eye_y - 4}" r="3" fill="#fff"/>'

    by = 0.12 if blink else 1
    o.append(f'<g transform="translate(0 {eye_y}) scale(1 {by}) translate(0 {-eye_y})">{eye(167)}{eye(233)}</g>')
    if k.get("wrinkles"):
        o.append(f'<path d="M128 206 l-10 -4 M128 214 l-10 2 M272 206 l10 -4 M272 214 l10 2" stroke="{k["shade"]}" stroke-width="4" stroke-linecap="round"/>')
    if k.get("glasses"):
        o.append(f'<g stroke="#3b2a1f" stroke-width="5" fill="#ffffff" fill-opacity="0.08"><circle cx="167" cy="{eye_y}" r="24"/><circle cx="233" cy="{eye_y}" r="24"/><path d="M191 {eye_y} L209 {eye_y}" fill="none"/></g>')
    o.append(f'<path d="M200 {eye_y + 12} Q192 {eye_y + 30} 204 {eye_y + 32}" stroke="{k["shade"]}" stroke-width="5" fill="none" stroke-linecap="round"/>')
    if k.get("mustache"):
        o.append(f'<path d="M176 {mouth_y - 10} Q200 {mouth_y - 24} 224 {mouth_y - 10} Q200 {mouth_y - 14} 176 {mouth_y - 10} Z" fill="{k["hairColor"]}"/>')

    y = mouth_y
    line = f'stroke="{INK}" fill="none" stroke-linecap="round"'
    if mouth_open and expr != "shock":
        o.append(f'<ellipse cx="200" cy="{y + 2}" rx="13" ry="9" fill="#6b2328"/>')
    elif expr == "smile":
        o.append(f'<path d="M172 {y - 6} Q200 {y + 30} 228 {y - 6} Z" fill="#6b2328"/>')
    elif expr == "shock":
        o.append(f'<ellipse cx="200" cy="{y + 4}" rx="13" ry="18" fill="#6b2328"/>')
    elif expr == "nervous":
        o.append(f'<path d="M176 {y} q6 -7 12 0 t12 0 t12 0 t12 0" {line} stroke-width="5"/>')
    elif expr == "sly":
        o.append(f'<path d="M182 {y + 2} Q206 {y + 8} 224 {y - 10}" {line} stroke-width="6"/>')
    elif expr == "sad":
        o.append(f'<path d="M180 {y + 8} Q200 {y - 8} 220 {y + 8}" {line} stroke-width="6"/>')
    elif expr == "angry":
        o.append(f'<path d="M182 {y + 4} L218 {y + 4}" {line} stroke-width="7"/>')
    elif expr == "determined":
        o.append(f'<path d="M184 {y + 2} Q200 {y + 6} 216 {y + 2}" {line} stroke-width="6"/>')
    elif expr == "sleepy":
        o.append(f'<ellipse cx="200" cy="{y + 2}" rx="7" ry="5" fill="#6b2328"/>')
    else:
        o.append(f'<path d="M182 {y} Q200 {y + 10} 218 {y}" {line} stroke-width="6"/>')
    if expr == "nervous":
        o.append('<path d="M290 150 Q300 172 290 180 Q280 172 290 150 Z" fill="#9fd8ff" stroke="#5aa9d6" stroke-width="2"/>')
    if k.get("scarf"):
        sc = k["scarf"]
        o.append(f'<path d="M146 282 Q200 318 254 282 L262 312 Q200 346 138 312 Z" fill="{sc}"/><path d="M226 312 L250 312 L262 392 L232 396 Z" fill="{sc}"/>'
                 '<path d="M232 380 L262 376 M234 366 L260 362" stroke="#000" stroke-opacity="0.2" stroke-width="4"/>')
    return "".join(o)


def character(cid, size, expr_fn, talk_fn, start, end, x=0.0, y=0.0, bob=True):
    """The character as a group of static SVG states (expression, mouth,
    blink), each on screen only for its own frames."""
    seed = len(cid) * 17
    s = size / 400

    def state(f):
        expr = expr_fn(f)
        talking = talk_fn(f) if talk_fn else False
        mouth = talking and (f // 4) % 2 == 0
        blink = (f + seed) % 100 < 4 and expr != "sleepy"
        return (expr, bool(mouth and expr != "shock"), bool(blink))

    layers = []
    run_start, cur = start, state(start)
    for f in range(start + 1, end + 1):
        nxt = state(f) if f < end else None
        if nxt != cur:
            expr, mouth, blink = cur
            aid = svg_asset(f"ch-{cid}-{expr}-{int(mouth)}{int(blink)}", character_svg(cid, expr, mouth, blink), 400, CHAR_H)
            layers.append(svg(aid, 400, CHAR_H, start=run_start, end=f))
            run_start, cur = f, nxt
    inner = group(layers, start=start, end=end, x=x, y=y)
    inner["transform"]["scale"] = s
    if bob:
        animate(inner, "transform.position.y", lambda f: y + math.sin((f + seed) / 16) * 3 * s, step=4, eps=0.05)
    return inner


def bust(cid, expr_fn, x, y, size, delay, dur, talk_fn=None, flip=False, tag=None):
    """Half-body portrait that springs up from below."""
    p = lambda f: spring(f - delay, 14, 130)
    start = max(0, delay)
    ch = character(cid, size, expr_fn, talk_fn, start, dur)
    if flip:
        ch = group([ch], size=(size, size), anchor=(0.5, 0.5), x=size / 2, y=size / 2)
        ch["transform"]["scale"] = {"x": -1, "y": 1}
    kids = [ch]
    if tag:
        kids.append(text(tag, MONO, 26, C["ink"], anchor=(0.5, 1), x=size / 2, y=size - 18,
                         bg=panel(CAST[cid][1], 6, (14, 4, 14, 4))))
    g = group(kids, start=start, end=dur, x=x, y=y)
    animate(g, "transform.position.y", lambda f: y + (1 - p(f)) * 300)
    animate(g, "opacity", lambda f: min(1, p(f) * 1.6))
    return g


# ------------------------------------------------------------------ small UI parts
def chapter_label(label, dur, start=0, end=None):
    end = dur if end is None else end
    p = lambda f: spring(f, 16, 140)
    pill = text(label.upper(), MONO, 30, C["white"], spacing=4, anchor=(0, 0.5), x=0, y=0,
                bg=panel("rgba(0,0,0,0.38)", 40, (50, 10, 22, 10), ("rgba(255,255,255,0.16)", 2)))
    dot = ellipse(14, 14, fill=C["accent"], anchor=(0.5, 0.5), x=31, y=0, start=start, end=end)
    animate(dot, "opacity", lambda f: 0.4 + (0.5 + 0.5 * math.sin(f / 6)) * 0.6, step=2, eps=0.01)
    g = group([pill, dot], start=start, end=end, x=56, y=118)
    animate(g, "transform.position.x", lambda f: 56 + (1 - p(f)) * -60)
    animate(g, "opacity", p)
    return g


def title(lines, top, size, dur, delay=0, stagger=6):
    """Big display title: each line pops in a little after the previous one."""
    out = []
    pitch = size * 0.95
    for i, segs in enumerate(lines):
        value = "".join(s[0] for s in segs)
        spans, pos = [], 0
        for seg in segs:
            if len(seg) > 1:
                spans.append({"start": pos, "end": pos + len(seg[0].rstrip()), "color": seg[1]})
            pos += len(seg[0])
        d = delay + i * stagger
        p = lambda f, d=d: spring(f - d, 11, 180)
        t = text(value, DISPLAY, size, C["white"], spacing=2, line_height=0.95, spans=spans or None,
                 anchor=(0.5, 0.5), x=W / 2, y=top + i * pitch + pitch / 2, start=max(0, d), end=dur,
                 shadow={"y": 8, "blur": 0, "color": "rgba(0,0,0,0.45)"})
        animate(t, "transform.scale", lambda f, p=p: 1.5 - 0.5 * p(f))
        animate(t, "opacity", lambda f, p=p: min(1, p(f) * 1.5))
        out.append(t)
    return out


def stamp(label, x, y, delay, dur, color=None, rotate=-8, size=64, end=None):
    color = color or C["accent"]
    end = dur if end is None else end
    p = lambda f: spring(f - delay, 10, 220)
    t = text(label, DISPLAY, size, color, spacing=3, anchor=(0.5, 0.5), x=x, y=y, start=delay, end=end,
             bg=panel("rgba(255,250,240,0.9)", 10, (22, 4, 22, 0), (color, 7)),
             shadow={"y": 6, "blur": 20, "color": "rgba(0,0,0,0.35)"})
    t["transform"]["rotation"] = rotate
    animate(t, "transform.scale", lambda f: 2.2 - 1.2 * p(f))
    animate(t, "opacity", lambda f: min(1, p(f) * 2))
    return t


def evidence_circle(x, y, w, h, delay, dur, color=None, start=None, end=None):
    """Hand-drawn ring that draws itself around a clue, then pulses."""
    start = delay if start is None else start
    end = dur if end is None else end
    e = ellipse(w, h, stroke={"color": color or C["accent"], "width": 8, "cap": "round", "trim_end": 0},
                anchor=(0.5, 0.5), x=x, y=y, start=start, end=end)
    e["transform"]["rotation"] = -12
    animate(e, "stroke.trim_end", lambda f: interp(f, [delay, delay + 14], [0, 1], in_out_cubic), eps=0.002)
    animate(e, "transform.scale", lambda f: 1 + 0.03 * math.sin((f - delay) / 4) if f > delay + 14 else 1, eps=0.001)
    return e


def time_badge(time, x, y, color, dur, delay=0, start=0, end=None):
    end = dur if end is None else end
    p = lambda f: spring(f - delay, 12, 160)
    pill = text(time, MONO, 34, color, anchor=(0, 0.5),
                bg=panel("rgba(0,0,0,0.55)", 40, (62, 8, 20, 8), (rgba(color, 0.33), 2)))
    ring = ellipse(24, 24, stroke={"color": color, "width": 3}, anchor=(0.5, 0.5), x=37, y=0)
    hands = path("M0 -7 L0 0 L5 3", stroke={"color": color, "width": 3, "cap": "round"}, x=37, y=0)
    g = group([pill, ring, hands], x=x, y=y + 29, start=max(start, 0), end=end)
    animate(g, "transform.scale", p)
    return g


def polaroid(x, y, w, h, caption, rotate, delay, dur, children, end=None):
    end = dur if end is None else end
    p = lambda f: spring(f - delay, 13, 130)
    total_h = h + 88
    photo = group([rect(w - 36, h, "#111111")] + children, size=(w - 36, h), clip=True, x=18, y=18)
    g = group([
        rect(w, total_h, C["paper"], 6, shadow={"y": 18, "blur": 40, "color": "rgba(0,0,0,0.5)"}),
        photo,
        text(caption, MONO, 34, C["ink"], anchor=(0.5, 1), x=w / 2, y=total_h - 14),
    ], size=(w, total_h), anchor=(0.5, 0.5), x=x + w / 2, y=y + total_h / 2, start=delay, end=end)
    animate(g, "transform.position.y", lambda f: y + total_h / 2 + (1 - p(f)) * 120)
    animate(g, "transform.rotation", lambda f: rotate + (1 - p(f)) * 10)
    animate(g, "opacity", p)
    return g


def crop(scale, left, top, children, w, h):
    g = group(children, size=(w, h), x=left, y=top)
    g["transform"]["scale"] = scale
    return g


def lightning(times, dur):
    o = lambda f: max([0.0] + [interp(f, [a, a + 2, a + 5, a + 7, a + 14], [0, 0.75, 0.1, 0.45, 0]) for a in times])
    start, end = max(0, min(times)), min(dur, max(times) + 15)
    r = rect(W, H, "#dfe9ff", start=start, end=end)
    return animate(r, "opacity", o)


def rain(dur, opacity, start=0, end=None, count=150):
    end = dur if end is None else end
    drops = []
    for _ in range(count):
        x0 = rng.random() * (W + 200) - 100
        sp = 38 + rng.random() * 26
        ln = 30 + rng.random() * 40
        y0 = rng.random() * H
        yf = lambda f, y0=y0, sp=sp: ((y0 + f * sp) % (H + 200)) - 100
        d = path(f"M0 0 L{-ln * 0.12:.2f} {ln:.2f}", stroke={"color": "#a9c3e8", "width": 2.5, "cap": "round"},
                 opacity=round(opacity * (0.5 + rng.random() * 0.5), 3), start=start, end=end)
        animate(d, "transform.position.y", yf, eps=0.5)
        animate(d, "transform.position.x", lambda f, x0=x0, yf=yf: x0 - yf(f) * 0.12, eps=0.5)
        drops.append(d)
    return group(drops, start=start, end=end)


# ------------------------------------------------------------------ subtitles
def parse_cue_text(raw):
    """'a *b c* d' -> clean text, [(start, end)] highlighted character ranges, word count."""
    words = raw.split(" ")
    clean_words, marks, inside = [], [], False
    for w in words:
        opens = w.count("*")
        hl = inside or opens > 0
        if opens % 2 == 1:
            inside = not inside
        clean_words.append(w.replace("*", ""))
        marks.append(hl)
    spans, pos, cur = [], 0, None
    for w, hl in zip(clean_words, marks):
        if hl:
            cur = [pos, pos + len(w)] if cur is None else [cur[0], pos + len(w)]
        elif cur is not None:
            spans.append(tuple(cur))
            cur = None
        pos += len(w) + 1
    if cur is not None:
        spans.append(tuple(cur))
    return " ".join(clean_words), spans, len(clean_words)


def subs(tl, top=1440):
    """Karaoke captions: words light up one by one, key words in colour."""
    out = []
    for ln in tl["lines"]:
        start, dur, who = ln["from"], ln["dur"], ln.get("who")
        clean, hl_spans, n = parse_cue_text(ln["text"])
        hl = CAST[who][1] if who else C["gold"]
        per = min(7, max(3, dur * 0.55 / n))
        starts = [round(i * per) for i in range(n)]
        kids, box_y = [], 0
        if who:
            kids.append(text(CAST[who][0], DISPLAY, 44, C["ink"], spacing=2, anchor=(0.5, 0), x=W / 2, y=0,
                             bg=panel(hl, 8, (18, 2, 18, 0))))
            box_y = 56 + 16
        cap = text(clean, SANS, 50, C["white"], weight=800, line_height=1.28, align="center", max_width=872, hug=True,
                   spans=[{"start": a, "end": b, "color": hl} for a, b in hl_spans] or None,
                   reveal={"unit": "word", "count": 0, "hidden_color": "rgba(255,255,255,0.3)"},
                   bg=panel("rgba(8,10,16,0.72)", 24, (34, 20, 34, 20)),
                   anchor=(0.5, 0), x=W / 2, y=box_y, start=start, end=start + dur)
        animate(cap, "text.reveal", lambda f, s=start, st=starts: sum(1 for t in st if f - s >= t))
        kids.append(cap)
        g = group(kids, start=start, end=start + dur, y=top)
        enter = lambda f, s=start: interp(f - s, [0, 7], [0, 1], out_cubic)
        animate(g, "transform.position.y", lambda f, e=enter: top + (1 - e(f)) * 24)
        animate(g, "opacity", lambda f, e=enter, s=start, d=dur: e(f) * interp(f - s, [d - 5, d], [1, 0]))
        out.append(g)
    return out


def timeline(cues, tail=0):
    """Scene timing without voice: each line lasts its `dur`."""
    t, lines = 0, []
    for c in cues:
        t += c.get("gap", 0)
        lines.append({"from": t, "dur": c["dur"], "who": c.get("who"), "text": c["text"]})
        t += c["dur"]
    SCENE["dur"] = t + tail  # layers built after this default to the scene's length
    return {"lines": lines, "dur": t + tail}


def at(tl, i, offset=0):
    return tl["lines"][i]["from"] + offset


def speaking(tl, who):
    """Mouth moves for the first 60% of the speaker's line."""
    def fn(f):
        for ln in tl["lines"]:
            if ln["from"] <= f < ln["from"] + ln["dur"]:
                return ln["who"] == who and f - ln["from"] < ln["dur"] * 0.6
        return False
    return fn


def scene(dur, layers):
    g = group(layers, size=(W, H))
    animate(g, "opacity", lambda f: interp(f, [0, 8, dur - 6, dur], [0, 1, 1, 0]))
    for child in g["layers"]:
        finalize(child)
    g.pop("_range"), g.pop("_timed")
    g["_dur"] = dur
    return g


# ------------------------------------------------------------------ illustrations
def watch_shape(spin=0, hands=True):
    o = ['<rect x="-46" y="-170" width="92" height="340" rx="30" fill="#6a3d22"/>',
         '<path d="M-46 -150 L46 -150 M-46 150 L46 150" stroke="#4e2c17" stroke-width="6"/>',
         f'<circle r="92" fill="#b88a2e"/><circle r="84" fill="{C["gold"]}"/><circle r="68" fill="#fbf6e9"/>']
    for i in range(12):
        big = i % 3 == 0
        o.append(f'<line x1="0" y1="-58" x2="0" y2="{-46 if big else -52}" stroke="#3a2a14" stroke-width="{6 if big else 3}" transform="rotate({i * 30})"/>')
    if hands:
        o.append(f'<line x1="0" y1="0" x2="0" y2="-38" stroke="#2a2014" stroke-width="7" stroke-linecap="round" transform="rotate({300 + spin / 12})"/>'
                 f'<line x1="0" y1="0" x2="0" y2="-54" stroke="#2a2014" stroke-width="5" stroke-linecap="round" transform="rotate({spin})"/>'
                 '<circle r="7" fill="#b88a2e"/>')
    o.append('<rect x="88" y="-10" width="18" height="20" rx="4" fill="#b88a2e"/>')
    return "".join(o)


def glow_def(gid, color):
    return (f'<radialGradient id="{gid}"><stop offset="0%" stop-color="{color}" stop-opacity="0.7"/>'
            f'<stop offset="100%" stop-color="{color}" stop-opacity="0"/></radialGradient>')


def house(lights_fn, dur, top, start=0, end=None, fog=True):
    """Homestay exterior. `lights_fn(frame)` is 0..1 for the homestay's own
    lamps; the neighbour's house always stays lit (the clue)."""
    end = dur if end is None else end
    on = C["lampOn"]
    back = [f'<defs><linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="#0a0f1c"/><stop offset="100%" stop-color="#1b2438"/></linearGradient>{glow_def("lampGlow", on)}</defs>',
            '<rect width="1080" height="1100" fill="url(#sky)"/>',
            '<path d="M0 520 C180 400 320 470 470 420 C640 360 820 450 1080 380 L1080 1100 L0 1100 Z" fill="#14202e"/>',
            '<path d="M0 640 C220 560 420 620 620 580 C820 540 940 600 1080 560 L1080 1100 L0 1100 Z" fill="#11261e"/>']
    for i in range(7):
        k = i * 34
        back.append(f'<path d="M0 {680 + k} C260 {610 + k} 520 {660 + k} 760 {620 + k} C900 {600 + k} 1000 {630 + k} 1080 {610 + k}" stroke="#1a3a2b" stroke-width="10" fill="none" stroke-linecap="round"/>')
    back.append('<path d="M0 860 L1080 860 L1080 1100 L0 1100 Z" fill="#0d1713"/><path d="M380 860 L460 860 L520 1100 L300 1100 Z" fill="#1a1f24"/>')
    back.append(f'<ellipse cx="860" cy="700" rx="220" ry="160" fill="url(#lampGlow)" opacity="0.35"/>'
                '<path d="M700 640 L860 540 L1020 640 Z" fill="#33405a"/><rect x="720" y="640" width="280" height="220" fill="#2a3242"/>'
                f'<rect x="750" y="680" width="70" height="70" fill="{on}"/><rect x="900" y="680" width="70" height="70" fill="{on}"/>'
                '<rect x="835" y="760" width="50" height="100" fill="#1b2130"/>'
                '<path d="M785 680 L785 750 M750 715 L820 715 M935 680 L935 750 M900 715 L970 715" stroke="#2a3242" stroke-width="5"/>')

    def windows(fill):
        w = [f'<circle cx="400" cy="530" r="38" fill="{fill}" stroke="#2c1d14" stroke-width="8"/>']
        for x in (210, 500):
            w.append(f'<rect x="{x}" y="660" width="90" height="100" fill="{fill}" stroke="#2c1d14" stroke-width="8"/>'
                     f'<path d="M{x + 45} 660 L{x + 45} 760 M{x} 710 L{x + 90} 710" stroke="#2c1d14" stroke-width="6"/>')
        return "".join(w)

    front = ['<path d="M130 600 L400 410 L670 600 Z" fill="#7a2f25"/><path d="M130 600 L400 410 L410 418 L150 600 Z" fill="#9a3d30"/>',
             '<rect x="170" y="598" width="460" height="262" fill="#4b3426"/>']
    for i in range(9):
        front.append(f'<line x1="170" x2="630" y1="{626 + i * 28}" y2="{626 + i * 28}" stroke="#3d2a1e" stroke-width="4"/>')
    front.append(windows(C["lampOff"]))
    front.append(f'<rect x="355" y="720" width="90" height="140" fill="#2c1d14"/><circle cx="430" cy="795" r="5" fill="{C["gold"]}"/>'
                 '<rect x="290" y="628" width="220" height="60" rx="6" fill="#d9c49a" stroke="#2c1d14" stroke-width="5"/>'
                 f'<text x="400" y="668" text-anchor="middle" font-family="{DISPLAY}" font-size="36" fill="#3a2614" letter-spacing="2">TERATAK KABUS</text>')

    kids = [svg(svg_asset("house-back", "".join(back), 1080, 1100), 1080, 1100)]
    glow = ellipse(600, 600, fill={"type": "radial", "stops": [{"offset": 0, "color": rgba(on, 0.7)}, {"offset": 1, "color": rgba(on, 0)}]},
                   anchor=(0.5, 0.5), x=400, y=700, start=start, end=end)
    glow["transform"]["scale"] = {"x": 1, "y": 220 / 300}
    animate(glow, "opacity", lambda f: 0.45 * lights_fn(f), eps=0.002)
    kids.append(glow)
    kids.append(svg(svg_asset("house-front", "".join(front), 1080, 1100), 1080, 1100))
    lit = svg(svg_asset("house-lit", windows(on), 1080, 1100), 1080, 1100, start=start, end=end)
    animate(lit, "opacity", lights_fn, eps=0.002)
    kids.append(lit)
    if fog:
        for i in range(3):
            e = ellipse(760, 100, fill="#c8d4e6", opacity=0.07, anchor=(0.5, 0.5), y=780 + i * 70, start=start, end=end)
            animate(e, "transform.position.x", lambda f, i=i: ((f * (0.6 + i * 0.3) + i * 400) % 1600) - 260, step=2, eps=0.3)
            kids.append(e)
    return group(kids, size=(1080, 1100), clip=True, y=top, start=start, end=end)


def house_static(lights):
    """Children for a still copy of the exterior (used inside polaroids)."""
    kids = house(lambda f: lights, SCENE["dur"], 0, fog=False)["layers"]
    for k in kids:
        k["_timed"] = False  # constant, so it simply follows its parent
    return kids


def lobby_svg(drawer="closed", dark=0):
    on = C["lampOn"]
    o = [f'<defs>{glow_def("lobbyLamp", on)}</defs>',
         '<rect width="1080" height="1100" fill="#3a3140"/><rect y="620" width="1080" height="300" fill="#2c2433"/>']
    for i in range(18):
        o.append(f'<line x1="{i * 64}" x2="{i * 64}" y1="620" y2="920" stroke="#251e2b" stroke-width="5"/>')
    o.append('<rect y="900" width="1080" height="200" fill="#3a2a20"/>')
    for i in range(6):
        o.append(f'<line x1="0" x2="1080" y1="{920 + i * 34}" y2="{920 + i * 34}" stroke="#2e2119" stroke-width="4"/>')
    o.append('<rect x="60" y="200" width="300" height="360" fill="#0d1424"/><path d="M60 470 C140 430 220 460 360 420 L360 560 L60 560 Z" fill="#11261e"/>'
             '<path d="M190 470 L260 420 L330 470 Z" fill="#33405a"/><rect x="200" y="470" width="120" height="80" fill="#2a3242"/>'
             f'<rect x="214" y="484" width="34" height="30" fill="{on}"/><rect x="272" y="484" width="34" height="30" fill="{on}"/>')
    r = random.Random(5)
    for _ in range(24):
        x, y = 60 + r.random() * 300, 200 + r.random() * 336
        o.append(f'<line x1="{x:.1f}" y1="{y:.1f}" x2="{x - 3:.1f}" y2="{min(560, y + 24):.1f}" stroke="#a9c3e8" stroke-width="2" opacity="0.5"/>')
    o.append('<rect x="60" y="200" width="300" height="360" fill="none" stroke="#5b4030" stroke-width="18"/>'
             '<path d="M210 200 L210 560 M60 380 L360 380" stroke="#5b4030" stroke-width="10"/>'
             '<path d="M40 186 L120 186 Q100 380 128 580 L40 580 Z" fill="#7a3a4a"/><path d="M380 186 L300 186 Q320 380 292 580 L380 580 Z" fill="#7a3a4a"/>')
    o.append('<rect x="850" y="360" width="190" height="540" fill="#4a3324" stroke="#2c1d14" stroke-width="10"/>'
             '<rect x="880" y="400" width="130" height="170" fill="#0d1424" stroke="#2c1d14" stroke-width="8"/>'
             f'<circle cx="1012" cy="660" r="9" fill="{C["gold"]}"/><rect x="830" y="892" width="230" height="18" fill="#6b4a32"/>')
    o.append('<path d="M720 760 L736 700 M760 760 L760 690 M800 760 L788 704" stroke="#111" stroke-width="8"/>'
             f'<path d="M736 700 q-14 -26 -30 -6" stroke="{CAST["aina"][1]}" stroke-width="8" fill="none" stroke-linecap="round"/>'
             f'<path d="M760 690 q-14 -26 -30 -6" stroke="{C["accent"]}" stroke-width="8" fill="none" stroke-linecap="round"/>'
             '<path d="M788 704 q-14 -26 -30 -6" stroke="#22314f" stroke-width="8" fill="none" stroke-linecap="round"/>'
             '<rect x="706" y="760" width="110" height="150" rx="10" fill="#5d6677"/><rect x="706" y="760" width="110" height="18" rx="6" fill="#7a8496"/>')
    o.append('<rect x="110" y="820" width="90" height="90" rx="8" fill="#8a5a3a"/>'
             '<path d="M155 820 C100 760 90 700 130 660 C150 720 150 760 155 820 Z" fill="#2f6b4a"/>'
             '<path d="M155 820 C200 760 230 690 190 640 C170 710 162 760 155 820 Z" fill="#3c8259"/>')
    o.append('<rect x="260" y="640" width="420" height="36" rx="6" fill="#7a5638"/><rect x="280" y="676" width="380" height="232" fill="#5a3d28"/>'
             '<rect x="280" y="676" width="380" height="232" fill="none" stroke="#3f2a1c" stroke-width="6"/>')
    if drawer == "closed":
        o.append(f'<rect x="380" y="712" width="180" height="64" rx="6" fill="#6b4a32" stroke="#3f2a1c" stroke-width="5"/><rect x="450" y="738" width="40" height="10" rx="5" fill="{C["gold"]}"/>')
    else:
        o.append('<rect x="380" y="712" width="180" height="64" rx="6" fill="#2a1c12"/>'
                 '<path d="M350 760 L590 760 L610 830 L330 830 Z" fill="#7a5638" stroke="#3f2a1c" stroke-width="5"/>'
                 '<path d="M360 768 L580 768 L592 812 L348 812 Z" fill="#3f2a1c"/>')
        if drawer == "watch":
            o.append(f'<g transform="translate(470 792) scale(0.55)">{watch_shape()}</g>')
    o.append(f'<path d="M560 640 Q590 596 620 640 Z" fill="{C["gold"]}"/><rect x="586" y="588" width="8" height="12" fill="{C["gold"]}"/>'
             '<rect x="300" y="618" width="120" height="22" rx="3" fill="#e8e1d2" transform="rotate(-4 360 630)"/>')
    o.append('<line x1="540" y1="0" x2="540" y2="120" stroke="#111" stroke-width="4"/><path d="M480 160 L600 160 L570 120 L510 120 Z" fill="#c78d3f"/>'
             f'<ellipse cx="540" cy="260" rx="360" ry="260" fill="url(#lobbyLamp)" opacity="{0.35 * (1 - dark)}"/>')
    if dark > 0:
        o.append(f'<rect width="1080" height="1100" fill="#04060b" opacity="{dark * 0.88}"/>'
                 f'<g opacity="{dark}"><rect x="214" y="484" width="34" height="30" fill="{on}"/><rect x="272" y="484" width="34" height="30" fill="{on}"/>'
                 '<ellipse cx="260" cy="500" rx="110" ry="70" fill="url(#lobbyLamp)" opacity="0.5"/></g>')
    return svg_asset(f"lobby-{drawer}-{int(dark)}", "".join(o), 1080, 1100)


def switchbox_svg():
    o = ['<rect width="1080" height="900" fill="#2c2433"/><rect x="190" y="140" width="700" height="620" rx="18" fill="#9aa1ad"/>'
         '<rect x="220" y="170" width="640" height="560" rx="10" fill="#c9ced6"/>',
         f'<text x="540" y="232" text-anchor="middle" font-family="{MONO}" font-size="34" fill="#3a3f4a">PAPAN SUIS · TERATAK KABUS</text>',
         '<g transform="translate(270 300)"><rect width="170" height="300" rx="10" fill="#2b2f38"/><rect x="50" y="60" width="70" height="140" rx="8" fill="#14161b"/>'
         f'<text x="85" y="250" text-anchor="middle" font-family="{DISPLAY}" font-size="40" fill="#fff">SUIS UTAMA</text>'
         f'<text x="85" y="44" text-anchor="middle" font-family="{MONO}" font-size="26" fill="#9aa1ad">ON</text>'
         f'<text x="85" y="286" text-anchor="middle" font-family="{MONO}" font-size="26" fill="#9aa1ad">OFF</text></g>']
    for i in range(6):
        o.append(f'<g transform="translate({480 + (i % 3) * 120} {310 + (i // 3) * 170})"><rect width="90" height="140" rx="8" fill="#e9ecf0" stroke="#8b919c" stroke-width="3"/>'
                 '<rect x="30" y="30" width="30" height="60" rx="4" fill="#3a3f4a"/><rect x="33" y="34" width="24" height="24" rx="3" fill="#cfd4dc"/></g>')
    return svg_asset("switchbox", "".join(o), 1080, 900)


def umbrella_stand_svg():
    def umbrella(x, color, wet, tag, tag_color):
        o = [f'<g transform="translate({x} 0)"><path d="M0 250 L-34 620 L34 620 Z" fill="{color}"/>',
             '<path d="M0 250 L-10 620 M0 250 L12 620" stroke="#000" stroke-opacity="0.25" stroke-width="4"/>',
             '<rect x="-5" y="620" width="10" height="120" fill="#222"/>',
             '<path d="M0 250 L0 190 q0 -40 -40 -40 q-26 0 -26 26" stroke="#2a2a2a" stroke-width="12" fill="none" stroke-linecap="round"/>']
        if wet:
            o.append('<path d="M-20 300 q-6 10 0 14 q6 -4 0 -14 Z M14 420 q-6 10 0 14 q6 -4 0 -14 Z M-6 520 q-6 10 0 14 q6 -4 0 -14 Z" fill="#cdefff" opacity="0.8"/>')
        o.append(f'<g transform="translate(0 110)"><rect x="-80" y="-34" width="160" height="52" rx="8" fill="{tag_color}"/>'
                 f'<text y="4" text-anchor="middle" font-family="{DISPLAY}" font-size="40" fill="{C["ink"]}" letter-spacing="2">{tag}</text></g></g>')
        return "".join(o)

    o = ['<rect width="1080" height="1000" fill="#2c2433"/><rect y="820" width="1080" height="180" fill="#3a2a20"/>',
         '<ellipse cx="300" cy="930" rx="150" ry="30" fill="#6fa8d6" opacity="0.55"/><ellipse cx="540" cy="935" rx="140" ry="28" fill="#6fa8d6" opacity="0.55"/>',
         umbrella(300, CAST["aina"][1], True, "AINA", CAST["aina"][1]),
         umbrella(540, C["accent"], True, "IKMAL", CAST["ikmal"][1]),
         umbrella(780, "#22314f", False, "DANIEL", CAST["daniel"][1]),
         '<ellipse cx="540" cy="884" rx="380" ry="22" fill="#000" opacity="0.35"/><path d="M190 700 L890 700 L870 880 L210 880 Z" fill="#5d6677"/>'
         '<path d="M190 700 L890 700 L885 740 L195 740 Z" fill="#4b5363"/>']
    for x in (300, 540, 780):
        o.append(f'<rect x="{x - 70}" y="760" width="140" height="100" rx="10" fill="#535b6b"/>')
    o.append('<rect x="176" y="690" width="728" height="22" rx="8" fill="#8a94a6"/>')
    return svg_asset("umbrella-stand", "".join(o), 1080, 1000)


def umbrella_stand(dur, top, start, end):
    """Two wet umbrellas dripping into puddles, one suspiciously dry."""
    kids = [svg(umbrella_stand_svg(), 1080, 1000)]
    for x in (300, 540):
        for i in range(3):
            d = path("M0 0 q-7 12 0 18 q7 -6 0 -18 Z", fill="#9fd8ff", x=x - 30 + i * 30, start=start, end=end)
            t = lambda f, i=i, x=x: (f + i * 11 + x) % 30
            animate(d, "transform.position.y", lambda f, t=t: 880 + t(f) * 1.2, eps=0.01)
            animate(d, "opacity", lambda f, t=t: 0.9 if t(f) < 24 else 0, eps=0.001)
            kids.append(d)
    return group(kids, size=(1080, 1000), clip=True, y=top, start=start, end=end)


def cake_svg():
    o = ['<ellipse cx="260" cy="460" rx="230" ry="36" fill="#e8e1d2"/><rect x="70" y="300" width="380" height="150" rx="20" fill="#7a4a2e"/>'
         '<path d="M70 320 Q110 360 150 320 T230 320 T310 320 T390 320 T450 320 L450 300 L70 300 Z" fill="#fff4e6"/>'
         '<rect x="110" y="190" width="300" height="120" rx="18" fill="#9a5d3a"/>'
         '<path d="M110 210 Q140 240 170 210 T230 210 T290 210 T350 210 T410 210 L410 190 L110 190 Z" fill="#fff4e6"/>']
    for x in (200, 260, 320):
        o.append(f'<rect x="{x - 7}" y="130" width="14" height="60" rx="4" fill="{CAST["makcik"][1]}"/>')
    o.append(f'<text x="260" y="400" text-anchor="middle" font-family="{DISPLAY}" font-size="50" fill="#fff4e6">SELAMAT HARI JADI</text>')
    return svg_asset("cake", "".join(o), 520, 520)


# ------------------------------------------------------------------ cards
CARD_W, CARD_H = 560, 696


def suspect_card(s, index, dur, expr_fn, talk_fn=None, start=0, end=None, label="SUSPEK"):
    """Cream ID card: portrait, name, alibi. Returned unscaled at (0, 0)."""
    end = dur if end is None else end
    cid, col = s["id"], CAST[s["id"]][1]
    portrait = group([
        rect(504, 400, "#1b2130"),
        rect(504, 400, {"type": "radial", "center": [0.5, 0.4], "radius": 0.69,
                        "stops": [{"offset": 0, "color": rgba(col, 0.4)}, {"offset": 0.45, "color": rgba(col, 0.133)}, {"offset": 1, "color": "#1b2130"}]}),
        character(cid, 400, expr_fn, talk_fn, start, end, x=52, y=0),
    ], size=(504, 400), clip=True, radius=8, x=28, y=76)
    return [
        rect(CARD_W, CARD_H, C["paper"], 14, shadow={"y": 24, "blur": 50, "color": "rgba(0,0,0,0.55)"}),
        path("M14 60 L14 18 Q14 6 23 6 Q32 6 32 18 L32 72 Q32 88 23 88 Q10 88 10 72 L10 30",
             stroke={"color": "#b9bec9", "width": 6, "cap": "round"}, x=250, y=-44),
        text(f"{label} #{index}", MONO, 28, C["inkSoft"], x=28, y=26),
        text(s["code"], MONO, 28, C["inkSoft"], anchor=(1, 0), x=532, y=26),
        portrait,
        text(CAST[cid][0], DISPLAY, 104, C["ink"], line_height=1.0, x=28, y=494),
        rect(160, 8, col, 4, x=28, y=602),
        text("ALIBI", DISPLAY, 34, C["paper"], x=28, y=628, bg=panel(C["ink"], 4, (12, 2, 12, 0))),
        text(s["alibi"], MONO, 28, C["ink"], anchor=(0, 0.5), x=118, y=650),
    ]


def mystery_card(trait, index, dur):
    stripes = ['<defs><clipPath id="c"><rect width="560" height="760" rx="14"/></clipPath></defs><g clip-path="url(#c)"><rect width="560" height="760" fill="#1a2030"/>']
    step = 36 * math.sqrt(2)
    for i in range(-20, 40):
        x = i * step
        stripes.append(f'<path d="M{x:.1f} 0 L{x + step / 2:.1f} 0 L{x + step / 2 - 760:.1f} 760 L{x - 760:.1f} 760 Z" fill="#20283a"/>')
    stripes.append('</g><rect x="2" y="2" width="556" height="756" rx="12" fill="none" stroke="#39445c" stroke-width="4"/>')
    q = text("?", DISPLAY, 330, C["accent"], line_height=1.0, anchor=(0.5, 0.5), x=280, y=176 + 165,
             shadow={"y": 10, "blur": 0, "color": "rgba(0,0,0,0.4)"})
    animate(q, "transform.rotation", lambda f: math.sin(f / 8) * 4, step=2, eps=0.02)
    return [
        rect(560, 760, "#1a2030", 14, shadow={"y": 24, "blur": 50, "color": "rgba(0,0,0,0.55)"}),
        svg(svg_asset("mystery-card", "".join(stripes), 560, 760), 560, 760),
        text(f"#{index}", MONO, 28, "#7d88a3", x=28, y=24),
        q,
        text(trait, DISPLAY, 60, C["white"], spacing=2, anchor=(0.5, 0), x=280, y=176 + 330,
             bg=panel("rgba(0,0,0,0.45)", 8, (26, 6, 26, 0))),
    ]


def case_file(title_text, rows, delay, dur, start=0, end=None, cpf=2):
    """Manila folder; its contents are typed out character by character."""
    end = dur if end is None else end
    p = lambda f: spring(f - delay, 14, 120)
    paper_h = max(520, 36 + 101 + len(rows) * 66 + 44)
    total_h = 70 + paper_h + 68
    kids = [
        rect(300, 86, "#c9a45c", 16),
        text("SULIT", MONO, 30, "#4a3a1c", x=26, y=16),
        rect(900, paper_h + 68, "#d8b674", 16, y=70, shadow={"y": 30, "blur": 60, "color": "rgba(0,0,0,0.55)"}),
        rect(20, 20, "#d8b674", y=70),
        rect(832, paper_h, C["paper"], 6, x=34, y=104),
    ]
    for k in range(1, int(paper_h // 65) + 1):
        if 65 * k - 2 < paper_h - 4:
            kids.append(rect(832, 2, "#cfc3aa", x=34, y=104 + 65 * k - 2))
    used = [0]

    def typed(value, **kw):
        offset = used[0]
        used[0] += len(value)
        t = text(value, reveal={"unit": "char", "count": 0}, start=start, end=end, **kw)
        animate(t, "text.reveal", lambda f, o=offset, n=len(value): float(max(0, min(n, math.floor(max(0, f - delay - 12) * cpf - o)))))
        return t

    kids.append(typed(title_text, font=DISPLAY, size=84, color=C["accent"], spacing=3, x=74, y=140))
    for i, (key, value) in enumerate(rows):
        y = 241 + 22 + i * 66
        kids.append(typed(key, font=MONO, size=38, color=C["inkSoft"], x=74, y=y))
        kids.append(typed(value, font=MONO, size=38, color=C["ink"], x=284, y=y))
    g = group(kids, size=(900, total_h), anchor=(0.5, 0.5), x=90 + 450, y=380 + total_h / 2, start=start, end=end)
    animate(g, "transform.position.y", lambda f: 380 + total_h / 2 + (1 - p(f)) * 300)
    animate(g, "transform.rotation", lambda f: -2 + (1 - p(f)) * -6)
    animate(g, "opacity", p)
    return g


# ------------------------------------------------------------------ scenes
def s1():
    tl = timeline([
        {"gap": 10, "dur": 82, "text": "Lima minit *bekalan elektrik* terputus..."},
        {"dur": 88, "text": "...dan seutas *jam emas* lenyap begitu saja."},
    ], 20)
    dur = tl["dur"]
    gone = at(tl, 1, 26) + 1  # first frame without the watch
    watch = group([
        svg(svg_asset("watch", f'<circle r="150" fill="{C["gold"]}" opacity="0.12"/>{watch_shape(hands=False)}', 400, 400, "-200 -200 400 400"),
            400, 400, x=-200, y=-200),
    ], x=540, y=900, end=gone)
    watch["transform"]["scale"] = 1.5
    for d, width, speed, base in (("M0 0 L0 -38", 7, 0.5, 300), ("M0 0 L0 -54", 5, 6, 0)):
        hand = path(d, stroke={"color": "#2a2014", "width": width, "cap": "round"}, end=gone)
        animate(hand, "transform.rotation", lambda f, s=speed, b=base: b + f * s)
        watch["layers"].append(hand)
    watch["layers"].append(ellipse(14, 14, fill="#b88a2e", anchor=(0.5, 0.5)))
    animate(watch, "transform.rotation", lambda f: math.sin(f / 14) * 6, step=2, eps=0.02)

    dark = rect(W, H, "#020305", start=at(tl, 1, 4), end=gone)
    animate(dark, "opacity", lambda f: interp(f, [at(tl, 1, 4), at(tl, 1, 8)], [0, 1]) * 0.92)
    q = text("?", DISPLAY, 460, C["accent"], anchor=(0.5, 0.5), x=540, y=916, start=gone, end=dur,
             shadow={"y": 14, "blur": 0, "color": "rgba(0,0,0,0.5)"})
    animate(q, "transform.scale", lambda f: spring(f - at(tl, 1, 30), 9, 200))
    return scene(dur, [
        rain(dur, 0.3),
        chapter_label("Kes #01 · Teratak Kabus", dur),
        *title([[("JAM EMAS HILANG",)], [("DALAM GELAP", C["accent"])]], 210, 150, dur),
        watch, dark, q,
        lightning([4, 60], dur),
        *subs(tl),
    ])


def s2():
    tl = timeline([
        {"gap": 6, "dur": 104, "text": "Cameron Highlands. *Hujan lebat* sejak pukul sembilan malam."},
        {"gap": 2, "dur": 98, "who": "aina", "text": "Eh Mal, homestay Mak Cik tiba-tiba *gelap*!"},
        {"dur": 34, "who": "ikmal", "text": "Hmm..."},
        {"dur": 62, "text": "Lima minit kemudian, lampu *menyala* semula."},
        {"dur": 84, "text": "Pukul 9:55, Ikmal dan sepupunya Aina sampai — *basah lencun*."},
    ], 10)
    dur = tl["dur"]
    a = lambda i, o=0: at(tl, i, o)

    def lights(f):
        if f < a(1, 12):
            return (0.2 if (f // 2) % 2 == 0 else 1) if f > a(1) else 1
        if f < a(3, 2):
            return 0
        return interp(f, [a(3, 2), a(3, 8)], [0, 1])

    badges = []
    for label, start, end in (("9:44 MLM", 0, a(1)), ("9:45 MLM", a(1), a(3)), ("9:50 MLM", a(3), a(4)), ("9:55 MLM", a(4), dur)):
        badges.append(time_badge(label, 56, 180, C["accent"] if label == "9:45 MLM" else C["white"], dur, start=start, end=end))
    return scene(dur, [
        house(lights, dur, 260),
        rain(dur, 0.42),
        chapter_label("Cameron Highlands", dur),
        *badges,
        bust("aina", lambda f: "neutral" if f < a(1) else "shock" if f < a(3) else "smile", -20, 1000, 420, a(1, -12), dur,
             speaking(tl, "aina"), tag="AINA · sepupu"),
        bust("ikmal", lambda f: "determined" if f < a(4) else "smile", 640, 1000, 420, a(1, -2), dur,
             speaking(tl, "ikmal"), flip=True, tag="IKMAL · 17 · detektif amatur"),
        lightning([30, a(2, -30)], dur),
        *subs(tl),
    ])


def s3():
    tl = timeline([
        {"gap": 6, "dur": 94, "who": "makcik", "text": "Ikmal! *Jam emas* arwah Pak Cik hilang!"},
        {"dur": 100, "who": "makcik", "text": "Pukul 9:30, Mak Cik simpan dalam *laci kaunter* ni."},
        {"dur": 110, "who": "makcik", "text": "Pukul 9:45 *blackout*. Lima minit lepas tu lampu menyala..."},
        {"gap": 10, "dur": 110, "who": "makcik", "text": "...laci dah *kosong*!"},
    ], 10)
    dur = tl["dur"]
    a = lambda i, o=0: at(tl, i, o)
    dim = rect(1080, 1100, "#000000", y=260)
    animate(dim, "opacity", lambda f: interp(f, [a(1, -4), a(1, 10)], [0, 0.6]))
    still = lambda aid, scale, left, top: [crop(scale, left, top, [svg(aid, 1080, 1100)], 1080, 1100)]
    return scene(dur, [
        svg(lobby_svg("empty"), 1080, 1100, y=260),
        dim,
        chapter_label("Lobi · 9:56 mlm", dur),
        polaroid(28, 330, 340, 290, "9:30", -5, a(1), dur, still(lobby_svg("watch"), 0.7, -180, -400)),
        polaroid(370, 300, 340, 290, "9:45", 3, a(2), dur, still(lobby_svg("closed", 1), 0.48, -20, -110)),
        polaroid(712, 340, 340, 290, "9:50", -2, a(3, -10), dur, still(lobby_svg("empty"), 0.7, -180, -400)),
        stamp("KOSONG", 880, 560, a(3, 6), dur, size=56),
        bust("makcik", lambda f: "shock" if f < a(1) else "sad" if f < a(3) else "shock", 560, 960, 480, 0, dur,
             speaking(tl, "makcik"), flip=True),
        *subs(tl),
    ])


def s4():
    tl = timeline([{"gap": 90, "dur": 120, "text": "Malam itu, hanya ada *tiga orang* lain di Teratak Kabus."}], 10)
    dur = tl["dur"]
    return scene(dur, [
        chapter_label("Fail kes", dur),
        case_file("FAIL KES #01", [("BARANG", "Jam emas arwah suami"), ("TKP", "Laci kaunter lobi"), ("MASA", "9:45 – 9:50 mlm"),
                                   ("KEADAAN", "Blackout 5 minit"), ("PEMILIK", "Mak Cik Rosnah")], 4, dur),
        *subs(tl),
    ])


SUSPECTS = [
    dict(id="zul", code="TK-01", trait="PALING BERMOTIF", alibi="TIDUR DALAM BILIK", stamp="HUTANG SEWA", stampColor=None, expr="sleepy",
         lines=["Pertama, yang *paling bermotif*.", "Encik Zul — dah *dua bulan* tak bayar sewa.", "Saya *tidur* dalam bilik. Tak dengar apa-apa pun."]),
    dict(id="lina", code="TK-02", trait="PALING GELABAH", alibi="DI DAPUR", stamp="MENCURIGAKAN", stampColor=None, expr="nervous",
         lines=["Kedua, yang *paling gelabah*.", "Kak Lina, anak saudara Mak Cik. Asyik *berpeluh*.", "S-saya kat *dapur*... buat air je!"]),
    dict(id="daniel", code="TK-03", trait="ALIBI PALING KUAT", alibi="DI LUAR 9:00–9:52", stamp="ADA ALIBI", stampColor="#2f9e6e", expr="smile",
         lines=["Ketiga, yang alibinya *paling kuat*.", "Daniel, jurugambar. Datang nak tangkap *gambar kabus*.",
                "Saya keluar dari pukul 9. *Balik 9:52*, sebelum korang sampai."]),
]


def mini_card(s, index, side, dur):
    """An already-revealed suspect, tucked at the edge and dimmed."""
    card = group(suspect_card(s, index, dur, lambda f: s["expr"]), size=(CARD_W, CARD_H))
    k = 360 / 560
    card["transform"]["scale"] = k
    st = stamp(s["stamp"], 180, 170, -100, dur, color=s["stampColor"], size=40)
    g = group([card, st, rect(CARD_W * k, CARD_H * k, "#000000", 14 * k, opacity=0.3)],
              size=(CARD_W, CARD_H), anchor=(0.5, 0.5), x=(-150 if side == "left" else 860) + CARD_W / 2, y=520 + CARD_H / 2)
    g["transform"]["rotation"] = -10 if side == "left" else 10
    return g


def s5(idx):
    s = SUSPECTS[idx]
    tl = timeline([
        {"gap": 4, "dur": 70, "text": s["lines"][0]},
        {"dur": 70, "text": s["lines"][1]},
        {"dur": 86, "who": s["id"], "text": s["lines"][2]},
    ], 10)
    dur = tl["dur"]
    flip_at = at(tl, 1, -4)
    angle = lambda f: interp(f, [flip_at, flip_at + 12], [0, 180])
    turn = next(f for f in range(dur) if angle(f) > 90)  # first frame showing the back
    enter = lambda f: spring(f, 14, 120)

    # A card flip without 3D: the card narrows to an edge, then widens as its other face.
    front = group(mystery_card(s["trait"], idx + 1, dur), size=(560, 760), anchor=(0.5, 0), x=280, end=turn)
    animate(front, "transform.scale.x", lambda f: math.cos(math.radians(angle(f))), eps=0.002)
    back = group(suspect_card(s, idx + 1, dur, lambda f: s["expr"], speaking(tl, s["id"]), start=turn, end=dur),
                 size=(CARD_W, CARD_H), anchor=(0.5, 0), x=280, start=turn, end=dur)
    animate(back, "transform.scale.x", lambda f: math.cos(math.radians(angle(f) - 180)), eps=0.002)
    holder = group([front, back], x=260, y=300)
    animate(holder, "transform.position.y", lambda f: 300 + (1 - enter(f)) * 500)
    return scene(dur, [
        chapter_label(f"Suspek {idx + 1}/3", dur),
        mini_card(SUSPECTS[0], 1, "left", dur) if idx >= 1 else None,
        mini_card(SUSPECTS[1], 2, "right", dur) if idx >= 2 else None,
        holder,
        stamp(s["stamp"], 540, 640, at(tl, 1, 22), dur, color=s["stampColor"]),
        *subs(tl),
    ])


def s6():
    tl = timeline([
        {"gap": 6, "dur": 78, "text": "Ikmal terus memeriksa *papan suis*."},
        {"dur": 80, "who": "aina", "text": "Masa blackout tadi, rumah *sebelah* terang je kan?"},
        {"dur": 76, "who": "ikmal", "text": "Betul. Jadi bukan TNB... ada orang *tutup suis* dari dalam."},
        {"gap": 4, "dur": 84, "text": "Kemudian, Ikmal terpandang *rak payung* di pintu."},
        {"dur": 80, "who": "aina", "text": "Payung kita masih *menitis*. Dah jadi lopak air."},
        {"dur": 72, "who": "ikmal", "text": "Tapi payung *yang itu*...?"},
    ], 10)
    dur = tl["dur"]
    a = lambda i, o=0: at(tl, i, o)
    cut = a(3, -4)  # switch board -> umbrella stand
    main_on = lambda f: interp(f, [a(2, 12), a(2, 20), a(2, 42), a(2, 50)], [1, 0, 0, 1])
    knob = rect(58, 56, C["accent"], 6, x=270 + 56, end=cut)
    animate(knob, "transform.position.y", lambda f: 300 + 60 + interp(main_on(f), [0, 1], [96, 24]))
    board = group([svg(switchbox_svg(), 1080, 900), knob], size=(1080, 900), x=-30, y=420, end=cut)
    board["transform"]["scale"] = 900 / 1080
    ring = evidence_circle(320, 170, 150, 140, a(1, 20), dur, C["gold"], end=cut)
    photo = polaroid(660, 240, 390, 280, "9:45 · dari luar", 4, a(1), dur,
                     [crop(0.574, -170, -220, house_static(0), 1080, 1100), ring], end=cut)
    return scene(dur, [
        chapter_label("Siasatan", dur),
        board, photo,
        evidence_circle(266, 795, 220, 300, a(2, 22), dur, end=cut),
        umbrella_stand(dur, 300, cut, dur),
        evidence_circle(780, 760, 230, 560, a(5, 6), dur),
        bust("ikmal", lambda f: "determined" if f < a(5) else "shock", -30, 1060, 380, 10, dur, speaking(tl, "ikmal")),
        bust("aina", lambda f: "neutral", 720, 1060, 380, a(1, -4), dur, speaking(tl, "aina"), flip=True),
        *subs(tl),
    ])


def s7():
    tl = timeline([{"gap": 40, "dur": 110, "who": "ikmal", "text": "Fakta *tak pernah* menipu."}], 10)
    dur = tl["dur"]
    rays = []
    for i in range(36):
        r = path("M0 -60 L22 -1400 L-22 -1400 Z", fill=C["insight"] if i % 2 == 0 else C["accent"], opacity=0.22 if i % 2 == 0 else 0.12)
        r["transform"]["rotation"] = i * 10
        rays.append(r)
    burst = group(rays, x=540, y=860)
    animate(burst, "transform.rotation", lambda f: f * 0.6)
    glint = path("M0 -56 L10 -10 L56 0 L10 10 L0 56 L-10 10 L-56 0 L-10 -10 Z", fill="#ffffff", x=330, y=340, start=18, end=35)
    animate(glint, "opacity", lambda f: interp(f, [18, 24, 34], [0, 1, 0]))
    hero = group([character("ikmal", 800, lambda f: "determined", speaking(tl, "ikmal"), 0, dur), glint],
                 size=(800, 800), anchor=(0.5, 0.6), x=140 + 400, y=520 + 480)
    animate(hero, "transform.scale", lambda f: interp(f, [0, dur], [1, 1.1]), eps=0.0005)
    flash = rect(W, H, "#ffffff", end=15)
    animate(flash, "opacity", lambda f: interp(f, [0, 3, 14], [1, 0.9, 0]))
    return scene(dur, [
        rect(W, H, "#05070c"),
        burst,
        *title([[("FAKTA",)], [("TAK PERNAH ", C["insight"]), ("MENIPU",)]], 170, 140, dur, delay=30),
        hero, flash,
        *subs(tl),
    ])


def s8():
    tl = timeline([
        {"gap": 4, "dur": 58, "who": "ikmal", "text": "Pencuri jam tu..."},
        {"dur": 62, "who": "ikmal", "text": "...awak, *Daniel*!"},
        {"dur": 70, "who": "ikmal", "text": "Masa blackout, rumah jiran *tetap terang*."},
        {"dur": 70, "who": "ikmal", "text": "Suis ditutup *dari dalam* supaya awak boleh curi dalam gelap."},
        {"dur": 80, "who": "ikmal", "text": "Hujan lebat sejak pukul 9. Tapi payung awak *kering*."},
        {"dur": 76, "who": "ikmal", "text": "Maknanya awak *tak pernah* keluar rumah pun."},
        {"dur": 80, "who": "ikmal", "text": "Dan payung tu tak boleh dibuka..."},
        {"dur": 100, "who": "ikmal", "text": "...sebab *jam emas* tu tersorok di dalamnya!"},
    ], 20)
    dur = tl["dur"]
    a = lambda i, o=0: at(tl, i, o)
    b1, b2, b3 = a(2), a(4), a(6)  # section boundaries
    out = []
    for label, start, end in (("Deduksi", 0, b1), ("Bukti 1 · Lampu jiran", b1, b2), ("Bukti 2 · Payung kering", b2, b3), ("Bukti 3 · Dalam payung", b3, dur)):
        out.append(chapter_label(label, dur, start, end))

    # --- 0: three cards, Daniel is singled out
    pick = lambda f: interp(f, [a(1, 2), a(1, 18)], [0, 1])
    cards = []
    for i, s in enumerate(SUSPECTS):
        is_d = s["id"] == "daniel"
        expr = (lambda f: "shock" if f > a(1, 8) else "smile") if is_d else (lambda f, e=s["expr"]: e)
        card = group(suspect_card(s, i + 1, dur, expr, end=b1), size=(CARD_W, CARD_H), end=b1)
        base_x = 50 + i * 340
        if is_d:
            animate(card, "transform.position.x", lambda f: interp(pick(f), [0, 1], [base_x, 250]))
            animate(card, "transform.position.y", lambda f: interp(pick(f), [0, 1], [460, 330]))
            animate(card, "transform.scale", lambda f: interp(pick(f), [0, 1], [300, 580]) / 560, eps=0.0005)
        else:
            card["transform"]["position"] = {"x": base_x, "y": 460}
            card["transform"]["scale"] = 300 / 560
            animate(card, "opacity", lambda f: 1 - pick(f) * 0.8)
        cards.append(card)
    out += cards
    out.append(stamp("PELAKU", 540, 620, a(1, 22), dur, size=110, rotate=-10, end=b1))

    # --- 1: the neighbour's lights stayed on
    out.append(house(lambda f: 0, dur, 280, b1, b2))
    out.append(rain(dur, 0.35, b1, b2))
    out.append(time_badge("9:45 MLM", 56, 180, C["accent"], dur, delay=b1, start=b1, end=b2))
    out.append(evidence_circle(860, 980, 380, 300, a(2, 16), dur, C["gold"], start=b1, end=b2))
    out.append(stamp("TERANG", 860, 760, a(2, 26), dur, color=C["gold"], size=60, rotate=6, end=b2))
    out.append(stamp("GELAP", 400, 680, a(2, 46), dur, size=60, end=b2))

    # --- 2: the dry umbrella
    out.append(umbrella_stand(dur, 300, b2, b3))
    out.append(evidence_circle(780, 760, 230, 560, a(4, 20), dur, start=b2, end=b3))
    out.append(stamp("BASAH", 300, 500, a(4, 12), dur, color="#3d8fd1", size=54, rotate=-6, end=b3))
    out.append(stamp("BASAH", 540, 500, a(4, 18), dur, color="#3d8fd1", size=54, rotate=4, end=b3))
    out.append(stamp("KERING", 780, 500, a(4, 36), dur, size=64, rotate=-8, end=b3))

    # --- 3: the umbrella opens and the watch falls out
    opened = lambda f: interp(f, [a(6, 20), a(6, 50)], [0, 1])
    drop = lambda f: interp(f, [a(7), a(7, 30)], [0, 1])
    top = lambda f: interp(opened(f), [0, 1], [250, 330])
    part = lambda layer: layer.update({"_timed": True, "_range": (b3, dur)}) or layer
    fallen = group([svg(svg_asset("watch-still", watch_shape(), 400, 400, "-200 -200 400 400"), 400, 400, x=-200, y=-200)],
                   start=a(7) + 1, end=dur)
    fallen["transform"]["scale"] = 0.7
    animate(fallen, "transform.position.y", lambda f: interp(drop(f), [0, 1], [420, 760]))
    animate(fallen, "transform.rotation", lambda f: interp(drop(f), [0, 1], [0, 200]))
    canopy = part(path("M0 -80 Q-300 -40 -300 120 Q-150 90 0 120 Q150 90 300 120 Q300 -40 0 -80 Z", fill="#22314f"))
    animate(canopy, "transform.scale.x", lambda f: interp(opened(f), [0, 1], [34, 300]) / 300, eps=0.001)
    animate(canopy, "transform.position.y", top)
    animate(canopy, "opacity", opened, eps=0.002)
    closed = part(path("M0 250 L-34 620 L34 620 Z", fill="#22314f"))
    animate(closed, "opacity", lambda f: 1 - opened(f), eps=0.002)
    shaft = part(rect(10, 560, "#222222", x=-5))
    animate(shaft, "transform.position.y", lambda f: top(f) - 90)
    handle = part(path("M0 470 q0 40 -40 40 q-26 0 -26 -26", stroke={"color": "#2a2a2a", "width": 12, "cap": "round"}))
    animate(handle, "transform.position.y", top)
    out.append(group([rect(1080, 1000, "#2c2433"), rect(1080, 180, "#3a2a20", y=820),
                      group([fallen, canopy, closed, shaft, handle], x=540)],
                     size=(1080, 1000), clip=True, y=300, start=b3, end=dur))
    out.append(evidence_circle(540, 1060, 300, 260, a(7, 34), dur, C["gold"]))
    out.append(stamp("BUKTI", 820, 860, a(7, 40), dur, size=70))
    out.append(bust("daniel", lambda f: "shock", -40, 1050, 360, a(7, 20), dur))
    return scene(dur, out + subs(tl))


def s9():
    tl = timeline([
        {"gap": 4, "dur": 76, "text": "Dan Kak Lina yang gelabah tadi?"},
        {"dur": 80, "who": "lina", "text": "Saya sorok *kek hari jadi* Mak Cik! Hehe..."},
        {"dur": 90, "who": "ikmal", "text": "Kes *selesai*."},
        {"gap": 10, "dur": 160, "text": "Kes seterusnya: *Misteri Asrama Lama*. Follow supaya tak terlepas!"},
    ], 30)
    dur = tl["dur"]
    a = lambda i, o=0: at(tl, i, o)
    p1, p2 = a(2), a(3, -10)
    out = [chapter_label("Penutup", dur, 0, p1), chapter_label("Kes selesai", dur, p1, p2), chapter_label("Kes seterusnya", dur, p2, dur)]

    k = 640 / 520
    flames = []
    for x in (200, 260, 320):
        fl = ellipse(20, 36, fill="#ffcf5a", anchor=(0.5, 0.5), x=x, y=118, end=p1)
        animate(fl, "transform.scale", lambda f: 1 + 0.12 * math.sin(f / 2.3), eps=0.004)
        flames.append(fl)
    cake = group([svg(cake_svg(), 520, 520)] + flames, size=(520, 520), x=220, y=330, end=p1)
    cake["transform"]["scale"] = k
    out.append(cake)
    lina = bust("lina", lambda f: "nervous" if f < a(1) else "smile", -20, 1000, 440, 6, p1, speaking(tl, "lina"))
    makcik = bust("makcik", lambda f: "neutral" if f < a(1, 10) else "smile", 640, 1000, 440, a(1, -20), p1, flip=True)
    out += [lina, makcik]

    hero = character("ikmal", 800, lambda f: "smile", speaking(tl, "ikmal"), p1, p2, x=140, y=480)
    out.append(hero)
    out.append(stamp("KES SELESAI", 540, 420, a(2, 6), dur, size=120, rotate=-6, end=p2))

    out.append(case_file("KES #02", [("LOKASI", "Asrama lama"), ("SUSPEK", "4 orang"), ("PELAKU", "???")], a(3, 2), dur, start=p2))
    out.append(stamp("AKAN DATANG", 760, 930, a(3, 70), dur, size=64, rotate=-8))
    d = a(3, 90)
    p = lambda f: spring(f - d, 9, 200)
    follow = group([
        rect(330, 104, "#a8302a", 52, anchor=(0.5, 0.5), y=10),
        text("FOLLOW", DISPLAY, 76, C["white"], spacing=4, anchor=(0.5, 0.5),
             bg=panel(C["accent"], 60, (64, 10, 64, 2)), shadow={"blur": 50, "color": rgba(C["accent"], 0.53)}),
    ], x=540, y=1252, start=d, end=dur)
    animate(follow, "transform.scale", lambda f: p(f) * (1 + 0.04 * math.sin((f - d) / 5)), eps=0.001)
    animate(follow, "opacity", p)
    out.append(follow)
    return scene(dur, out + subs(tl))


# ------------------------------------------------------------------ whole episode
def main():
    os.makedirs(SVG_DIR, exist_ok=True)
    scenes = [s1(), s2(), s3(), s4(), s5(0), s5(1), s5(2), s6(), s7(), s8(), s9()]
    total = sum(s["_dur"] for s in scenes)
    SCENE["dur"] = total  # the layers below span the whole episode

    layers = [
        rect(W, H, "#07090d"),
        # radial-gradient(120% 80% at 50% 35%): a circle, stretched into an ellipse
        (lambda r: (r["transform"].__setitem__("scale", {"x": 1, "y": 1536 / 1296}), r)[1])(
            rect(2592, 2592, {"type": "radial", "stops": [{"offset": 0, "color": C["bg3"]}, {"offset": 0.6, "color": C["bg"]}, {"offset": 1, "color": "#07090d"}]},
                 anchor=(0.5, 0.5), x=540, y=672)),
    ]
    for i in range(46):  # drifting dust
        x, y0 = rng.random() * W, rng.random() * H
        sp, r = 0.25 + rng.random() * 0.6, 1.2 + rng.random() * 2.6
        dot = ellipse(2 * r, 2 * r, fill="#cfd8ea", anchor=(0.5, 0.5))
        animate(dot, "transform.position.y", lambda f, y0=y0, sp=sp: ((y0 - f * sp) % H + H) % H, step=4, eps=0.3)
        animate(dot, "transform.position.x", lambda f, x=x, i=i: x + math.sin(f / 40 + i) * 8, step=8, eps=0.3)
        animate(dot, "opacity", lambda f, i=i: 0.12 + 0.25 * (0.5 + 0.5 * math.sin(f / 20 + i)), step=6, eps=0.01)
        layers.append(dot)

    offset = 0
    for s in scenes:
        s["start_frame"], s["end_frame"] = offset, offset + s.pop("_dur")
        offset += s["end_frame"] - s["start_frame"]
        layers.append(s)

    # vignette: radial-gradient(90% 70% at 50% 45%, transparent 55%, rgba(0,0,0,.6))
    vig = rect(1944, 1944, {"type": "radial", "stops": [{"offset": 0, "color": "rgba(0,0,0,0)"}, {"offset": 0.55, "color": "rgba(0,0,0,0)"}, {"offset": 1, "color": "rgba(0,0,0,0.6)"}]},
               anchor=(0.5, 0.5), x=540, y=864)
    vig["transform"]["scale"] = {"x": 1, "y": 1344 / 972}
    layers.append(vig)
    layers.append(rect(W, 10, "rgba(255,255,255,0.08)"))
    bar = L("rect", size={"width": 1, "height": 10}, fill=C["accent"], shadow={"blur": 18, "color": C["accent"]})
    scene_ids = {id(x) for x in scenes}
    bar["animations"] = [{"property": "size.width", "keyframes": [{"frame": 0, "value": 0.001}, {"frame": total - 1, "value": W}]}]
    layers.append(bar)

    fonts = [
        {"id": "bebas", "type": "font", "src": "assets/fonts/BebasNeue-Regular.ttf", "family": DISPLAY},
        {"id": "elite", "type": "font", "src": "assets/fonts/SpecialElite-Regular.ttf", "family": MONO},
        {"id": "jakarta", "type": "font", "src": "assets/fonts/PlusJakartaSans-ExtraBold.ttf", "family": SANS},
    ]
    template = {
        "schema_version": "1.0",
        "composition": {"id": "kes01", "name": "Kes #01 — Jam Emas Dalam Gelap", "width": W, "height": H, "fps": FPS,
                        "duration_frames": total, "background": "#07090d"},
        "assets": fonts + list(ASSETS.values()),
        "layers": [x if id(x) in scene_ids else finalize(x) for x in layers],
    }
    out = os.path.join(HERE, "template.json")
    with open(out, "w", encoding="utf-8") as f:
        json.dump(template, f, ensure_ascii=False, separators=(",", ":"))
    print(f"{total} frames ({total / FPS:.1f} s), {len(ASSETS)} SVGs, {os.path.getsize(out) / 1e6:.2f} MB -> {out}")


if __name__ == "__main__":
    main()
