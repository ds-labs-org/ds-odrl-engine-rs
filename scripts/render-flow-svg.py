#!/usr/bin/env python3
"""Render docs/diagrams/02-dataspace-integration-flow.svg.

    python3 scripts/render-flow-svg.py > docs/diagrams/02-dataspace-integration-flow.svg

A sequence diagram, so it is a data-flow diagram rather than a component
diagram: hand-authored data below, no RDF source (docs/diagrams/01-* is the
hive: component diagram). Edit LANES / EVENTS, re-render, do not edit the SVG.

Honesty rule for this diagram: every phase says whether it is implemented or
proposed. Phase 1 is what ds-catalog-broker-rs does today (odrl_filter.rs);
phase 2 has its adapter and engine but no connector hook wired; phase 3 is
assurance and is never in a request's path.
"""
from xml.sax.saxutils import escape

W = 1000
LANES = [  # key, x, title, subtitle, palette
    ("consumer", 92, "Consumer", "connector + caller identity", "a"),
    ("broker", 322, "Provider", "catalog broker / connector", "b"),
    ("adapter", 552, "dsp-odrl-adapter", "JSON-LD offer to WirePolicy", "c"),
    ("engine", 762, "engine", "native crate or wasm module", "b"),
    ("n3", 918, "n3-enforcer", "N3 rules on eyeron", "d"),
]
X = {k: x for k, x, *_ in LANES}

# ("phase", title, status "impl"|"prop"|"assure", subtitle)
# ("msg", from, to, label, dashed)  ("note", lane, [lines])
EVENTS = [
    ("phase", "1 · Catalog visibility", "impl", "implemented in ds-catalog-broker-rs (odrl_filter.rs)"),
    ("msg", "consumer", "broker", "GET /catalog · Bearer JWT", False),
    ("note", "broker", ["verify the token; JWT claims and host context", "become engine Claims"]),
    ("msg", "broker", "engine", "Request{dataset_id, action, config, one policy, claims}", False),
    ("note", "engine", ["decide(): deny-overrides,", "open/closed, duties"]),
    ("msg", "engine", "broker", "Response{Allow | Deny, reason, duties}", True),
    ("note", "broker", ["OR across a dataset's alternative offers;", "hide what is not entitled (also management", "API and SPARQL query rewriting)"]),
    ("msg", "broker", "consumer", "catalog: only the entitled datasets", True),
    ("phase", "2 · Contract negotiation", "prop", "adapter and engine exist; a connector hook is proposed (case study §5.4)"),
    ("msg", "consumer", "broker", "ContractRequestMessage (JSON-LD offer)", False),
    ("msg", "broker", "adapter", "ingest(JSON-LD)", False),
    ("note", "adapter", ["expand against bundled @contexts,", "no network; an unknown @context", "is a hard error, never an empty policy"]),
    ("msg", "adapter", "broker", "WirePolicy", True),
    ("msg", "broker", "engine", "Request{…, claims from the caller's verified identity}", False),
    ("msg", "engine", "broker", "Allow | Deny + duties (advise or deny mode)", True),
    ("msg", "broker", "consumer", "ContractAgreementMessage, or termination", True),
    ("phase", "3 · Assurance", "assure", "not in any request's path: CI, and the ds42.org demonstrator"),
    ("msg", "n3", "engine", "same Request, second opinion (68/68 agree)", False),
]

ROW, NOTE_LINE = 44, 14
out = []
w = out.append


def wrap_ok(s):
    return escape(s)


y = 150.0
body = []
phases = []  # (y_top, y_bottom, title, status, subtitle)
cur_phase = None
for ev in EVENTS:
    kind = ev[0]
    if kind == "phase":
        if cur_phase:
            phases.append((*cur_phase, y - 16))
        y += 10
        cur_phase = (y - 26, ev[1], ev[2], ev[3])
        y += 16
    elif kind == "msg":
        _, a, b, label, dashed = ev
        body.append(("msg", y, a, b, label, dashed))
        y += ROW
    else:
        _, lane, lines = ev
        h = len(lines) * NOTE_LINE + 12
        body.append(("note", y - 12, lane, lines, h))
        y += h + 6
phases.append((*cur_phase, y - 10))
H = y + 30

w(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H:.0f}" role="img" aria-label="Sequence: how ds-odrl-engine-rs sits in a dataspace. A consumer calls the provider\'s catalog broker with a bearer token; the broker maps the token to claims and asks the engine, per policy, whether the caller may see each dataset, and returns a filtered catalog. For contract negotiation the adapter turns a JSON-LD offer into a wire policy for the same engine, which is proposed rather than wired. A reasoner-backed enforcer cross-checks the engine outside the request path.">')
w("""<style>
.fl-bg { fill: #ffffff; }
.fl-title { fill: #0f172a; font: 600 16px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.fl-sub { fill: #475569; font: 11px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; }
.fl-lane { font: 600 12.5px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.fl-lanesub { fill: #475569; font: 10.5px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.fl-life { stroke: #94a3b8; stroke-width: 1; stroke-dasharray: 3 4; }
.fl-msg { stroke: #334155; stroke-width: 1.5; fill: none; }
.fl-ret { stroke: #64748b; stroke-width: 1.4; stroke-dasharray: 5 3; fill: none; }
.fl-head { fill: #334155; }
.fl-label { fill: #0f172a; font: 11px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; paint-order: stroke; stroke: #ffffff; stroke-width: 3.5px; stroke-linejoin: round; }
.fl-note { fill: #f8fafc; stroke: #cbd5e1; stroke-width: 1; }
.fl-notetext { fill: #1e293b; font: 10.5px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; text-anchor: middle; }
.fl-phase-impl { fill: #ecfdf5; stroke: #10b981; } .fl-pt-impl { fill: #047857; }
.fl-phase-prop { fill: #fffbeb; stroke: #f59e0b; } .fl-pt-prop { fill: #b45309; }
.fl-phase-assure { fill: #f5f3ff; stroke: #8b5cf6; } .fl-pt-assure { fill: #6d28d9; }
.fl-phasetitle { font: 600 12.5px -apple-system, Segoe UI, Helvetica, Arial, sans-serif; }
.fl-box { stroke-width: 1.5; }
.fl-ba { fill: #eff6ff; stroke: #2563eb; } .fl-ta { fill: #1d4ed8; }
.fl-bb { fill: #f0fdfa; stroke: #0f766e; } .fl-tb { fill: #0f766e; }
.fl-bc { fill: #fffbeb; stroke: #b45309; } .fl-tc { fill: #b45309; }
.fl-bd { fill: #f5f3ff; stroke: #6d28d9; } .fl-td { fill: #6d28d9; }
@media (prefers-color-scheme: dark) {
  .fl-bg { fill: #0f172a; }
  .fl-title, .fl-label { fill: #f1f5f9; }
  .fl-sub, .fl-lanesub { fill: #cbd5e1; }
  .fl-label { stroke: #0f172a; }
  .fl-life { stroke: #475569; }
  .fl-msg { stroke: #cbd5e1; } .fl-head { fill: #cbd5e1; } .fl-ret { stroke: #94a3b8; }
  .fl-note { fill: #1e293b; stroke: #475569; } .fl-notetext { fill: #e2e8f0; }
  .fl-phase-impl { fill: #064e3b55; stroke: #34d399; } .fl-pt-impl { fill: #6ee7b7; }
  .fl-phase-prop { fill: #78350f55; stroke: #fbbf24; } .fl-pt-prop { fill: #fcd34d; }
  .fl-phase-assure { fill: #4c1d9555; stroke: #a78bfa; } .fl-pt-assure { fill: #c4b5fd; }
  .fl-ba { fill: #172554; stroke: #60a5fa; } .fl-ta { fill: #93c5fd; }
  .fl-bb { fill: #042f2e; stroke: #2dd4bf; } .fl-tb { fill: #5eead4; }
  .fl-bc { fill: #451a03; stroke: #fbbf24; } .fl-tc { fill: #fcd34d; }
  .fl-bd { fill: #2e1065; stroke: #a78bfa; } .fl-td { fill: #c4b5fd; }
}
</style>""")
w('<defs><marker id="fl-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto"><path class="fl-head" d="M0,0 L10,5 L0,10 z"/></marker></defs>')
w(f'<rect class="fl-bg" width="{W}" height="{H:.0f}" rx="8"/>')
w(f'<text class="fl-title" x="{W/2:.0f}" y="28">Where the engine sits in a dataspace</text>')
w(f'<text class="fl-sub" x="{W/2:.0f}" y="46" style="text-anchor:middle">A caller reaches a provider over the Dataspace Protocol; the provider asks the engine what the caller may do. Solid: request. Dashed: response.</text>')

# phase bands first (behind everything)
for top, title, status, sub, bottom in phases:
    w(f'<rect class="fl-phase-{status}" x="8" y="{top:.0f}" width="{W-16}" height="{bottom-top:.0f}" rx="6" fill-opacity="0.55" stroke-width="1"/>')
    w(f'<text class="fl-phasetitle fl-pt-{status}" x="20" y="{top+17:.0f}">{escape(title)}</text>')
    w(f'<text class="fl-sub" x="{20 + 8 * len(title) + 8:.0f}" y="{top+17:.0f}">{escape(sub)}</text>')

# lifelines and headers
for key, x, title, sub, pal in LANES:
    w(f'<line class="fl-life" x1="{x}" y1="112" x2="{x}" y2="{phases[-1][4]:.0f}"/>')
    w(f'<rect class="fl-box fl-b{pal}" x="{x-75}" y="66" width="150" height="44" rx="8"/>')
    w(f'<text class="fl-lane fl-t{pal}" x="{x}" y="86">{escape(title)}</text>')
    w(f'<text class="fl-lanesub" x="{x}" y="101">{escape(sub)}</text>')

for item in body:
    if item[0] == "msg":
        _, my, a, b, label, dashed = item
        x1, x2 = X[a], X[b]
        d = 1 if x2 > x1 else -1
        cls = "fl-ret" if dashed else "fl-msg"
        w(f'<line class="{cls}" x1="{x1 + 5*d}" y1="{my:.0f}" x2="{x2 - 3*d}" y2="{my:.0f}" marker-end="url(#fl-arrow)"/>')
        w(f'<text class="fl-label" x="{(x1 + x2)/2:.0f}" y="{my - 6:.0f}">{escape(label)}</text>')
    else:
        _, ny, lane, lines, h = item
        nw = max(len(l) for l in lines) * 5.9 + 20
        x = X[lane]
        w(f'<rect class="fl-note" x="{x - nw/2:.0f}" y="{ny:.0f}" width="{nw:.0f}" height="{h}" rx="5"/>')
        for i, line in enumerate(lines):
            w(f'<text class="fl-notetext" x="{x}" y="{ny + 18 + i*NOTE_LINE:.0f}">{escape(line)}</text>')

w("</svg>")
print("\n".join(out))
