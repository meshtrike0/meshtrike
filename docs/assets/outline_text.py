"""Turn the <text> in the README artwork into vector outlines.

GitHub shows SVGs as images, and images cannot load web fonts, so text would fall
back to whatever font the viewer has. This shapes each <text> with HarfBuzz using
the website's own fonts (website/fonts, SIL OFL) and replaces it with <path>s,
so the artwork looks the same everywhere.

    pip install fonttools brotli uharfbuzz
    python docs/assets/outline_text.py      # docs/assets/src/*.svg -> docs/assets/*.svg
"""
import io
import re
import pathlib
import xml.etree.ElementTree as ET

import uharfbuzz as hb
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = pathlib.Path(__file__).resolve().parents[2]
FONTS = {
    "Bricolage Grotesque": ROOT / "website/fonts/bricolage-grotesque-latin.woff2",
    "Geist": ROOT / "website/fonts/geist-latin.woff2",
}
SVG = "http://www.w3.org/2000/svg"
INHERIT = ("font-family", "font-size", "font-weight", "letter-spacing", "text-anchor", "fill")
ET.register_namespace("", SVG)

_faces = {}


def face(family):
    if family not in _faces:
        font = TTFont(FONTS[family])
        font.flavor = None
        raw = io.BytesIO()
        font.save(raw)
        _faces[family] = hb.Face(raw.getvalue())
    return _faces[family]


def outline(text, family, size, weight, spacing, anchor, x, y):
    f = hb.Font(face(family))
    f.set_variations({"wght": weight})
    upem = face(family).upem
    buf = hb.Buffer()
    buf.add_str(text)
    buf.guess_segment_properties()
    hb.shape(f, buf, {"kern": True, "liga": True})
    s = size / upem
    glyphs, pen_x = [], 0.0
    for info, pos in zip(buf.glyph_infos, buf.glyph_positions):
        glyphs.append((info.codepoint, pen_x + pos.x_offset * s, pos.y_offset * s))
        pen_x += pos.x_advance * s + spacing
    width = pen_x - spacing
    x0 = x - (width / 2 if anchor == "middle" else width if anchor == "end" else 0)
    svg_pen = SVGPathPen(None)
    for gid, gx, gy in glyphs:
        f.draw_glyph_with_pen(gid, TransformPen(svg_pen, (s, 0, 0, -s, x0 + gx, y - gy)))
    return re.sub(r"-?\d+\.\d+", lambda m: f"{float(m.group()):.1f}".rstrip("0").rstrip("."), svg_pen.getCommands())


def walk(el, ctx):
    ctx = dict(ctx, **{k: el.get(k) for k in INHERIT if el.get(k) is not None})
    for i, child in enumerate(list(el)):
        if child.tag == f"{{{SVG}}}text":
            c = dict(ctx, **{k: child.get(k) for k in INHERIT if child.get(k) is not None})
            family = c["font-family"].split(",")[0].strip().strip("'\"")
            d = outline(
                "".join(child.itertext()), family, float(c["font-size"]), float(c.get("font-weight", 400)),
                float(c.get("letter-spacing", 0)), c.get("text-anchor", "start"),
                float(child.get("x", 0)), float(child.get("y", 0)),
            )
            path = ET.Element(f"{{{SVG}}}path", {"d": d, "fill": c.get("fill", "#000")})
            el.remove(child)
            el.insert(i, path)
        else:
            walk(child, ctx)
    for k in ("font-family", "font-size", "font-weight", "letter-spacing", "text-anchor"):
        el.attrib.pop(k, None)


for src in sorted((ROOT / "docs/assets/src").glob("*.svg")):
    tree = ET.parse(src)
    walk(tree.getroot(), {})
    out = ROOT / "docs/assets" / src.name
    tree.write(out, encoding="unicode")
    print(f"{src.relative_to(ROOT)} -> {out.relative_to(ROOT)}")
