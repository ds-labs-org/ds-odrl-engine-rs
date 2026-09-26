#!/usr/bin/env python3
"""Render a hive: (ds-honeycomb-editor-rs) standalone diagram to a static SVG.

    python3 scripts/render-hive-svg.py docs/diagrams/01-engine-internals.ttl \
        > docs/diagrams/01-engine-internals.svg

The Turtle file is the source of truth; the SVG is a build product. Lattice
geometry is the odd-r pointy-top convention of honeycomb-core's
`Lattice::centre` (x = step*(col + 0.5*(row mod 2)), y = pitch*row, with
pitch = step*sqrt(3)/2), so a diagram lays out here exactly as the editor
would draw it. Needs only rdflib (a dev-time tool, not a build dependency).

Styling is deliberately not in the vocabulary (it refuses to know colours),
so this renderer owns it: one colour per group, keyed by hive:slug below.
Class names are prefixed `hv-` so the SVG can be inlined into a page next to
other SVGs without their rules colliding.
"""
import math
import sys
from xml.sax.saxutils import escape

from rdflib import Graph, Namespace, RDFS

HIVE = Namespace("https://semantic.ds-labs.org/vocab/honeycomb#")
R, GAP = 42.0, 1.34
STEP = math.sqrt(3) * R * GAP
PITCH = STEP * math.sqrt(3) / 2
PAD = 10.0  # how far a group's region grows past its tiles

# group slug -> palette index (light fill/stroke pairs live in the CSS)
GROUP_CLASS = {"hosts": "a", "core": "b", "adapters": "c", "verify": "d"}
# groups whose heading sits below their tiles (top would collide with a neighbour)
LABEL_BELOW = {"adapters", "core"}


def centre(col, row):
    return STEP * (col + 0.5 * (row % 2)), PITCH * row


def hex_path(cx, cy, r):
    dx, half = r * math.sqrt(3) / 2, r / 2
    pts = [(cx, cy - r), (cx + dx, cy - half), (cx + dx, cy + half),
           (cx, cy + r), (cx - dx, cy + half), (cx - dx, cy - half)]
    return "M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in pts) + " Z"


def wrap(label, width=11):
    """Break at hyphens/spaces into lines of at most `width` characters."""
    parts, out, cur = [], [], ""
    for token in label.replace("-", "-\0").replace(" ", " \0").split("\0"):
        parts.append(token)
    for token in parts:
        if cur and len(cur.rstrip()) + len(token.rstrip()) > width:
            out.append(cur.rstrip())
            cur = token
        else:
            cur += token
    if cur:
        out.append(cur.rstrip())
    return out


def one(g, s, p):
    v = g.value(s, p)
    return str(v) if v is not None else ""


def main(path):
    g = Graph()
    g.parse(path, format="turtle")
    diagram = next(g.subjects(HIVE.slug, None) if False else g.subjects(predicate=HIVE.placement))
    title = one(g, diagram, RDFS.label)
    comment = one(g, diagram, RDFS.comment)
    note = one(g, diagram, HIVE.note)

    tiles = {}
    for pl in g.objects(diagram, HIVE.placement):
        tile = g.value(pl, HIVE.tile)
        group = g.value(pl, HIVE.inGroup)
        col, row = int(g.value(pl, HIVE.col)), int(g.value(pl, HIVE.row))
        x, y = centre(col, row)
        tiles[pl] = dict(
            x=x, y=y, label=one(g, tile, RDFS.label), tip=one(g, tile, RDFS.comment),
            slug=one(g, tile, HIVE.slug), group=one(g, group, HIVE.slug) if group else "",
        )
    groups = {}
    for grp in g.objects(diagram, HIVE.group):
        groups[str(grp)] = dict(slug=one(g, grp, HIVE.slug), label=one(g, grp, RDFS.label),
                                note=one(g, grp, HIVE.note))

    xs = [t["x"] for t in tiles.values()]
    ys = [t["y"] for t in tiles.values()]
    margin_x, top, bottom = R + PAD + 24, 74.0, 78.0
    ox, oy = margin_x - min(xs), top + R + PAD - min(ys)
    width = max(xs) - min(xs) + 2 * margin_x
    height = max(ys) - min(ys) + top + bottom + 2 * (R + PAD)

    out = []
    w = out.append
    w(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.0f} {height:.0f}" role="img" '
      f'aria-label="{escape(title)}. {escape(comment)} A static rendering of the honeycomb-vocabulary source next to this file.">')
    w(CSS)
    w(f'<rect class="hv-bg" width="{width:.0f}" height="{height:.0f}" rx="8"/>')
    w(f'<text class="hv-title" x="{width/2:.0f}" y="30">{escape(title)}</text>')
    w(f'<text class="hv-caption" x="{width/2:.0f}" y="50">{escape(note)}</text>')

    # group regions: the union of hexagons grown by PAD, drawn behind everything
    for gid, grp in groups.items():
        cls = GROUP_CLASS.get(grp["slug"], "a")
        members = [t for t in tiles.values() if t["group"] == grp["slug"]]
        if not members:
            continue
        w(f'<g class="hv-region hv-g{cls}">')
        for t in members:
            w(f'<path d="{hex_path(t["x"] + ox, t["y"] + oy, R + PAD)}"/>')
        w("</g>")
    # group labels, above the topmost tile of each group
    for gid, grp in groups.items():
        cls = GROUP_CLASS.get(grp["slug"], "a")
        members = [t for t in tiles.values() if t["group"] == grp["slug"]]
        if not members:
            continue
        below = grp["slug"] in LABEL_BELOW
        edge_y = max(t["y"] for t in members) if below else min(t["y"] for t in members)
        edge = [t for t in members if abs(t["y"] - edge_y) < 1e-6]
        lx = sum(t["x"] for t in edge) / len(edge) + ox
        ly = edge_y + oy + (R + PAD + 15 if below else -(R + PAD + 5))
        w(f'<text class="hv-grouplabel hv-t{cls}" x="{lx:.1f}" y="{ly:.1f}">{escape(grp["label"])}</text>')

    # links
    w("<defs>")
    w('<marker id="hv-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">'
      '<path class="hv-arrowhead" d="M0,0 L10,5 L0,10 z"/></marker>')
    w("</defs>")
    labels = []
    drawn = set()  # (target, text): a second link into one tile with the same label stays a tooltip
    for link in g.subjects(HIVE["from"], None):
        a, b = tiles[g.value(link, HIVE["from"])], tiles[g.value(link, HIVE.to)]
        ax, ay, bx, by = a["x"] + ox, a["y"] + oy, b["x"] + ox, b["y"] + oy
        dist = math.hypot(bx - ax, by - ay)
        ux, uy = (bx - ax) / dist, (by - ay) / dist
        trim = R * 0.87 + 3
        sx, sy, ex, ey = ax + ux * trim, ay + uy * trim, bx - ux * (trim + 1), by - uy * (trim + 1)
        arc = g.value(link, HIVE.routing) == HIVE.arc
        text = one(g, link, RDFS.label)
        if arc:
            nx, ny = -uy, ux
            bend = dist * 0.22
            mx, my = (sx + ex) / 2 + nx * bend, (sy + ey) / 2 + ny * bend
            w(f'<path class="hv-link" d="M{sx:.1f},{sy:.1f} Q{mx:.1f},{my:.1f} {ex:.1f},{ey:.1f}" marker-end="url(#hv-arrow)"><title>{escape(text)}</title></path>')
            lxp, lyp = 0.25 * sx + 0.5 * mx + 0.25 * ex, 0.25 * sy + 0.5 * my + 0.25 * ey
        else:
            w(f'<line class="hv-link" x1="{sx:.1f}" y1="{sy:.1f}" x2="{ex:.1f}" y2="{ey:.1f}" marker-end="url(#hv-arrow)"><title>{escape(text)}</title></line>')
            lxp, lyp = (sx + ex) / 2, (sy + ey) / 2
        # neighbours share an edge: a label there would sit on tile ink, so it
        # stays a tooltip; longer links carry it on the line
        key = (g.value(link, HIVE.to), text)
        if text and dist > STEP * 1.25 and key not in drawn:
            drawn.add(key)
            labels.append((lxp, lyp, text))

    # tiles
    for t in tiles.values():
        cls = GROUP_CLASS.get(t["group"], "a")
        x, y = t["x"] + ox, t["y"] + oy
        w(f'<g class="hv-tile hv-g{cls}"><title>{escape(t["tip"])}</title><path d="{hex_path(x, y, R)}"/>')
        lines = wrap(t["label"])
        y0 = y - (len(lines) - 1) * 6.5 + 4
        for i, line in enumerate(lines):
            w(f'<text class="hv-label" x="{x:.1f}" y="{y0 + i * 13:.1f}">{escape(line)}</text>')
        w("</g>")

    for lx, ly, text in labels:
        w(f'<text class="hv-linklabel" x="{lx:.1f}" y="{ly - 4:.1f}">{escape(text)}</text>')

    words, lines, cur = comment.split(), [], ""
    for word in words:
        if len(cur) + len(word) + 1 > 118:
            lines.append(cur)
            cur = word
        else:
            cur = f"{cur} {word}".strip()
    lines.append(cur)
    for i, line in enumerate(lines):
        w(f'<text class="hv-caption" x="{width/2:.0f}" y="{height - 12 - (len(lines) - 1 - i) * 14:.0f}">{escape(line)}</text>')
    w("</svg>")
    sys.stdout.write("\n".join(out) + "\n")


CSS = """<style>
.hv-bg { fill: #ffffff; }
.hv-title { fill: #0f172a; font: 600 16px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.hv-caption { fill: #475569; font: 11px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.hv-grouplabel { font: 600 12.5px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.hv-label { fill: #0f172a; font: 11px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.hv-linklabel { fill: #334155; font: 10.5px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; paint-order: stroke; stroke: #ffffff; stroke-width: 3px; stroke-linejoin: round; }
.hv-link { stroke: #475569; stroke-width: 1.5; fill: none; }
.hv-arrowhead { fill: #475569; }
.hv-region path { stroke: none; }
.hv-tile path { stroke-width: 1.5; }
.hv-ga.hv-region path { fill: #dbeafe; } .hv-ga.hv-tile path { fill: #eff6ff; stroke: #2563eb; } .hv-ta { fill: #1d4ed8; }
.hv-gb.hv-region path { fill: #ccfbf1; } .hv-gb.hv-tile path { fill: #f0fdfa; stroke: #0f766e; } .hv-tb { fill: #0f766e; }
.hv-gc.hv-region path { fill: #fef3c7; } .hv-gc.hv-tile path { fill: #fffbeb; stroke: #b45309; } .hv-tc { fill: #b45309; }
.hv-gd.hv-region path { fill: #ede9fe; } .hv-gd.hv-tile path { fill: #f5f3ff; stroke: #6d28d9; } .hv-td { fill: #6d28d9; }
@media (prefers-color-scheme: dark) {
  .hv-bg { fill: #0f172a; }
  .hv-title, .hv-label { fill: #f1f5f9; }
  .hv-caption, .hv-linklabel { fill: #cbd5e1; }
  .hv-linklabel { stroke: #0f172a; }
  .hv-link { stroke: #94a3b8; } .hv-arrowhead { fill: #94a3b8; }
  .hv-ga.hv-region path { fill: #1e3a8a55; } .hv-ga.hv-tile path { fill: #172554; stroke: #60a5fa; } .hv-ta { fill: #93c5fd; }
  .hv-gb.hv-region path { fill: #134e4a55; } .hv-gb.hv-tile path { fill: #042f2e; stroke: #2dd4bf; } .hv-tb { fill: #5eead4; }
  .hv-gc.hv-region path { fill: #78350f55; } .hv-gc.hv-tile path { fill: #451a03; stroke: #fbbf24; } .hv-tc { fill: #fcd34d; }
  .hv-gd.hv-region path { fill: #4c1d9555; } .hv-gd.hv-tile path { fill: #2e1065; stroke: #a78bfa; } .hv-td { fill: #c4b5fd; }
}
</style>"""

if __name__ == "__main__":
    main(sys.argv[1])
