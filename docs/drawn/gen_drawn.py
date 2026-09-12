"""Build "Nimblecube, Drawn": the geometry diagram page.

    python3 docs/drawn/gen_drawn.py                  # writes docs/nimblecube-drawn.html (offline)
    python3 docs/drawn/gen_drawn.py --fragment F     # also writes an Artifact-ready fragment to F
    python3 docs/drawn/gen_drawn.py --fetch-fonts    # re-download fonts.css from Google Fonts

Every coordinate is computed here, never hand-placed, so change the numbers below and rebuild
rather than editing the output file. Files next to this script:

    template.html   page text, CSS and colours, with {{SVG1}}..{{SVG5}} where figures go
    fonts.css       the three typefaces embedded as data URIs, so the page works offline

Where to change what:

    Distance chart (figure 2)   the SETTINGS block just below; try DIM = 1024 or 256
    Cube (figure 1)             YAW / PITCH rotate it, PATH picks the highlighted corners
    Huddle map (figure 3)       huddle(...) calls set centre, count and spread per huddle
    Three moves (figure 4)      point positions inside the "figure 4" section
    Host vs device (figure 5)   the two box lists inside the "figure 5" section
    Symmetry (figure 6)         V is the XOR vector, TRI the triangle; keep V == TRI[0]
                                (placed on the page after the three moves)
    Diagonal view (figures 7, 8) TOP picks which one-bit corner points up
                                (placed on the page after the symmetry section)

The prose and captions in template.html are static text; they describe the default settings.
"""
import argparse, base64, itertools, math, pathlib, random, re, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
TEMPLATE, FONTS = HERE / "template.html", HERE / "fonts.css"
OUT = HERE.parent / "nimblecube-drawn.html"

# ---------- SETTINGS for the distance chart (figure 2) ----------
DIM = 4096                       # bits per hypervector, DIM_BITS in nimblecube-core/src/hv.rs
NOISE_FLIP_FRACTION = 100 / 4096  # fraction of bits a normal reading differs from its baseline
REJECT_FRACTION = 250 / 4096      # RejectNet radius + margin, as a fraction of DIM
MEASURED_ANOMALY = 2043           # from the ESP32 RejectNet run; only shown when DIM == 4096
# Derived, do not edit: unrelated corners sit at DIM/2 with sd sqrt(DIM)/2 (binomial, p = 0.5),
# normal readings sit at DIM*p with sd sqrt(DIM*p*(1-p)).
UNRELATED_MEAN, UNRELATED_SD = DIM / 2, math.sqrt(DIM) / 2
NORMAL_MEAN = DIM * NOISE_FLIP_FRACTION
NORMAL_SD = math.sqrt(DIM * NOISE_FLIP_FRACTION * (1 - NOISE_FLIP_FRACTION))
REJECT = DIM * REJECT_FRACTION

f = lambda v: f"{v:.1f}"

# ---------- figure 1: a 3-bit cube, orthographic, with its circumscribed sphere ----------
YAW, PITCH, S, CX, CY = math.radians(35), math.radians(24), 75, 230, 175
PATH = ["001", "011", "111"]
def proj(p):
    x, y, z = p
    x1 = x * math.cos(YAW) + z * math.sin(YAW); z1 = -x * math.sin(YAW) + z * math.cos(YAW)
    y2 = y * math.cos(PITCH) - z1 * math.sin(PITCH); z2 = y * math.sin(PITCH) + z1 * math.cos(PITCH)
    return CX + S * x1, CY - S * y2, z2
bits = lambda c: "".join("1" if v > 0 else "0" for v in c)
P = {bits(c): proj(c) for c in itertools.product([-1, 1], repeat=3)}
HIDDEN = min(P, key=lambda k: P[k][2])          # farthest corner, drawn hollow
R = S * math.sqrt(3)
edges = [(a, b) for a in P for b in P if a < b and sum(x != y for x, y in zip(a, b)) == 1]
on_path = {tuple(sorted(e)) for e in zip(PATH, PATH[1:])}

fig1 = [f'<circle cx="{CX}" cy="{CY}" r="{f(R)}" class="f-sphere s-line" stroke-width="1.2"/>',
        f'<ellipse cx="{CX}" cy="{CY}" rx="{f(R)}" ry="34" class="s-line" stroke-dasharray="3 4"/>',
        f'<line x1="{CX}" y1="{CY}" x2="{f(P["010"][0])}" y2="{f(P["010"][1])}" class="s-line" stroke-dasharray="2 3"/>',
        f'<circle cx="{CX}" cy="{CY}" r="2.5" class="f-muted"/>',
        '<text x="168" y="131" class="t-muted" font-style="italic">r</text>']
for a, b in edges:
    if tuple(sorted((a, b))) in on_path:
        continue
    dash = ' stroke-dasharray="4 4"' if HIDDEN in (a, b) else ""
    fig1.append(f'<line x1="{f(P[a][0])}" y1="{f(P[a][1])}" x2="{f(P[b][0])}" y2="{f(P[b][1])}" class="s-ink" stroke-width="1.4"{dash}/>')
pts = " ".join(f"{f(P[k][0])},{f(P[k][1])}" for k in PATH)
fig1.append(f'<polyline points="{pts}" class="s-acc" stroke-width="3.2" stroke-linejoin="round"/>')
for k, (x, y, _) in P.items():
    if k == HIDDEN:
        fig1.append(f'<circle cx="{f(x)}" cy="{f(y)}" r="5" class="f-ground s-line" stroke-width="1.5"/>')
    else:
        cls = "f-acc" if k in PATH else "f-ink"
        fig1.append(f'<circle cx="{f(x)}" cy="{f(y)}" r="5.2" class="{cls}"/>')
fig1 += [f'<text x="{f(P["001"][0])}" y="306" text-anchor="middle" class="t-acc">001</text>',
         f'<text x="{f(P["111"][0] + 11)}" y="{f(P["111"][1] + 4)}" class="t-acc">111</text>',
         f'<text x="{f(P["110"][0] + 10)}" y="{f(P["110"][1] - 8)}">110</text>',
         '<text x="256" y="172" class="t-acc">2 edges</text>']

# ---------- figure 2: distance distribution, to scale on 0..DIM ----------
L, RGT, BASE, H = 50, 870, 190, 120
xs = lambda d: L + d * (RGT - L) / DIM
def bell(mu, sd):
    pts = []
    for i in range(41):
        d = mu - 4 * sd + i * (8 * sd / 40)
        pts.append(f"{f(xs(d))},{f(BASE - H * math.exp(-0.5 * ((d - mu) / sd) ** 2))}")
    return f"M{f(xs(mu - 4 * sd))},{BASE} L" + " L".join(pts) + f" L{f(xs(mu + 4 * sd))},{BASE} Z"
fig2 = [f'<path d="{bell(NORMAL_MEAN, NORMAL_SD)}" class="f-acc"/>',
        f'<path d="{bell(UNRELATED_MEAN, UNRELATED_SD)}" class="f-warm"/>',
        f'<line x1="{L}" y1="{BASE}" x2="{RGT}" y2="{BASE}" class="s-ink" stroke-width="1.2"/>']
for t in (0, DIM // 4, DIM // 2, 3 * DIM // 4, DIM):
    fig2 += [f'<line x1="{f(xs(t))}" y1="{BASE}" x2="{f(xs(t))}" y2="{BASE + 6}" class="s-ink"/>',
             f'<text x="{f(xs(t))}" y="{BASE + 22}" text-anchor="middle">{t}</text>']
sigma_gap = (UNRELATED_MEAN - REJECT) / UNRELATED_SD
second = (f"unknown reading measured {MEASURED_ANOMALY}" if DIM == 4096
          else f"reject line is {sigma_gap:.0f} sd below this spike")
fig2 += [f'<line x1="{f(xs(REJECT))}" y1="62" x2="{f(xs(REJECT))}" y2="{BASE}" class="s-acc" stroke-width="1.5" stroke-dasharray="4 3"/>',
         f'<text x="{f(xs(REJECT) + 7)}" y="84" class="t-acc">reject line: {REJECT:.0f}</text>',
         f'<text x="{f(xs(NORMAL_MEAN) - 12)}" y="50" class="t-acc">normal readings: about {NORMAL_MEAN:.0f}</text>',
         f'<text x="{f(xs(UNRELATED_MEAN))}" y="30" text-anchor="middle" class="t-warm">everything unrelated: {UNRELATED_MEAN:.0f} ± {UNRELATED_SD:.0f}</text>',
         f'<text x="{f(xs(UNRELATED_MEAN))}" y="50" text-anchor="middle" class="t-muted">{second}</text>',
         f'<text x="{f((xs(REJECT) + xs(UNRELATED_MEAN - 4 * UNRELATED_SD)) / 2)}" y="150" text-anchor="middle" class="t-muted" font-style="italic">almost nothing lives out here</text>',
         f'<circle cx="{f(xs(DIM))}" cy="{BASE}" r="3.5" class="f-muted"/>',
         f'<text x="{f(xs(DIM) - 4)}" y="150" text-anchor="end" class="t-muted">{DIM}: the opposite corner</text>',
         f'<text x="{(L + RGT) // 2}" y="{BASE + 48}" text-anchor="middle" class="t-sans t-muted">Hamming distance from your baseline, in bits</text>']

# ---------- figure 3: huddles and the net, schematic map ----------
rng = random.Random(7)
def huddle(cx, cy, n, spread, cls, r=4.2):
    out = []
    for _ in range(n):
        a, d = rng.uniform(0, 2 * math.pi), abs(rng.gauss(0, spread))
        out.append(f'<circle cx="{f(cx + d * math.cos(a))}" cy="{f(cy + d * math.sin(a))}" r="{r}" class="{cls}"/>')
    return out
NX, NY = 250, 205
fig3 = ['<rect x="10" y="10" width="900" height="370" rx="18" class="f-panel"/>',
        '<text x="32" y="40" class="t-sans t-muted">a small patch of the sphere, flattened</text>',
        f'<circle cx="{NX}" cy="{NY}" r="80" class="f-acc-soft s-acc" stroke-width="1.6" stroke-dasharray="6 5"/>']
fig3 += huddle(NX, NY, 16, 22, "f-acc")
fig3 += [f'<circle cx="{NX}" cy="{NY}" r="8" class="f-ground s-acc" stroke-width="2.6"/>',
         f'<text x="{NX}" y="112" text-anchor="middle" class="t-title">normal huddle</text>',
         f'<text x="{NX}" y="306" text-anchor="middle" class="t-acc">RejectNet: radius 100 + margin 150</text>',
         f'<text x="{NX}" y="324" text-anchor="middle" class="t-muted">ring = the bundle, middle of the huddle</text>',
         '<circle cx="292" cy="182" r="6" class="f-ground s-ink" stroke-width="2"/>',
         '<line x1="299" y1="180" x2="344" y2="170" class="s-ink"/>',
         '<text x="350" y="174">new reading, d = 96: inside, normal</text>']
fig3 += huddle(585, 108, 12, 18, "f-muted")
fig3 += ['<text x="585" y="62" text-anchor="middle" class="t-title">fan on</text>',
         '<text x="585" y="80" text-anchor="middle" class="t-muted">another learned normal</text>']
fig3 += huddle(775, 238, 13, 19, "f-warm")
fig3 += ['<text x="775" y="186" text-anchor="middle" class="t-title">smoke</text>',
         '<text x="775" y="300" text-anchor="middle" class="t-warm">known failure, from the library</text>',
         '<line x1="490" y1="306" x2="510" y2="326" class="s-ink" stroke-width="2.6"/>',
         '<line x1="510" y1="306" x2="490" y2="326" class="s-ink" stroke-width="2.6"/>',
         f'<line x1="494" y1="310" x2="{NX + 10}" y2="{NY + 6}" class="s-ink" stroke-dasharray="3 4" marker-end="url(#f3-arr)"/>',
         '<text x="392" y="292">d = 2043 → rejected in one compare</text>',
         '<text x="500" y="352" text-anchor="middle" class="t-muted">unknown reading</text>']

# ---------- figure 4: the three moves ----------
fig4 = []
for i, (title, sub) in enumerate([("bundle", "one point close to all"),
                                  ("bind", "far from both inputs"),
                                  ("permute", "rotate: marks order")]):
    x0 = 10 + i * 308
    fig4 += [f'<rect x="{x0}" y="10" width="284" height="210" rx="14" class="f-panel"/>',
             f'<text x="{x0 + 22}" y="42" class="t-title">{title}</text>',
             f'<text x="{x0 + 22}" y="200" class="t-muted">{sub}</text>']
tri = [(80, 96), (116, 158), (206, 112)]
cxb, cyb = sum(p[0] for p in tri) / 3, sum(p[1] for p in tri) / 3
for x, y in tri:
    fig4 += [f'<line x1="{x}" y1="{y}" x2="{f(cxb)}" y2="{f(cyb)}" class="s-line" stroke-width="1.4"/>',
             f'<circle cx="{x}" cy="{y}" r="6" class="f-acc"/>']
fig4 += [f'<circle cx="{f(cxb)}" cy="{f(cyb)}" r="8" class="f-ground s-acc" stroke-width="2.6"/>',
         '<circle cx="372" cy="96" r="6" class="f-ink"/><text x="372" y="80" text-anchor="middle">A</text>',
         '<circle cx="396" cy="156" r="6" class="f-ink"/><text x="372" y="162" text-anchor="middle">B</text>',
         '<line x1="380" y1="98" x2="542" y2="122" class="s-ink" marker-end="url(#f4-arr)"/>',
         '<line x1="404" y1="153" x2="542" y2="128" class="s-ink" marker-end="url(#f4-arr)"/>',
         '<text x="468" y="100" class="t-muted">XOR</text>',
         '<circle cx="556" cy="125" r="7" class="f-acc"/><text x="556" y="152" text-anchor="middle" class="t-acc">≈ 2048 away</text>']
pcx, pcy, pr = 768, 122, 52
fig4 += [f'<circle cx="{pcx}" cy="{pcy}" r="{pr}" class="s-line" stroke-dasharray="3 4"/>',
         f'<path d="M{pcx - pr},{pcy} A{pr},{pr} 0 0,1 {pcx},{pcy - pr}" class="s-acc" stroke-width="2.4" marker-end="url(#f4-arr-acc)"/>',
         f'<circle cx="{pcx - pr}" cy="{pcy}" r="6" class="f-ink"/>',
         f'<circle cx="{pcx}" cy="{pcy - pr}" r="6" class="f-acc"/>',
         f'<text x="{pcx + 14}" y="{pcy - pr + 4}" class="t-acc">shift by 1</text>']

# ---------- figure 5: host versus device ----------
COL = [38, 214, 390, 566, 742]; BW, BH = 140, 58
def box(x, y, title, sub, sub_cls="t-muted", stroke="box"):
    return [f'<rect x="{x}" y="{y}" width="{BW}" height="{BH}" rx="9" class="{stroke}"/>',
            f'<text x="{x + BW / 2}" y="{y + 25}" text-anchor="middle" class="t-sans t-strong">{title}</text>',
            f'<text x="{x + BW / 2}" y="{y + 44}" text-anchor="middle" class="{sub_cls} t-small">{sub}</text>']
HY, DY = 52, 226
fig5 = ['<text x="38" y="34" class="t-title">host: your PC</text>',
        f'<text x="38" y="{DY - 18}" class="t-title">device: ESP32-S3</text>',
        '<line x1="20" y1="182" x2="900" y2="182" class="s-line" stroke-dasharray="6 5"/>',
        '<text x="38" y="146" class="t-muted t-small">trained, floating point, needs lots of data</text>',
        f'<text x="38" y="{DY + BH + 34}" class="t-muted t-small">integer only, learns from a few examples</text>']
for x, t, s, sc in zip(COL, ["text or document", "embedding model", "SimHash", "hypervector"],
                       ["input", "768 floats", "607 ms on chip", "512 B"],
                       ["t-muted", "t-muted", "t-warm", "t-muted"]):
    fig5 += box(x, HY, t, s, sc, "box-warm" if t == "SimHash" else "box")
for x, t, s in zip(COL, ["MQ-2 sensor", "features", "codebook", "RejectNet", "store, net tree"],
                   ["32 samples, 320 ms", "mean · peak · slope", "~2 µs lookup", "7.8 µs, 1 compare", "5.4 ms at 12,000"]):
    fig5 += box(x, DY, t, s)
for row in (HY, DY):
    n = 4 if row == HY else 5
    for i in range(n - 1):
        fig5.append(f'<line x1="{COL[i] + BW + 3}" y1="{row + BH / 2}" x2="{COL[i + 1] - 5}" y2="{row + BH / 2}" class="s-ink" marker-end="url(#f5-arr)"/>')
hx = COL[3] + BW / 2; dx = COL[4] + BW / 2
fig5 += [f'<path d="M{hx},{HY + BH + 2} L{hx},158 L{dx},158 L{dx},{DY - 6}" class="s-acc" stroke-width="2" marker-end="url(#f5-arr-acc)"/>',
         f'<text x="{(hx + dx) / 2}" y="150" text-anchor="middle" class="t-acc">enroll: 512 B per item</text>']

# ---------- figure 6: XOR moves everything, every distance stays ----------
xor = lambda a, b: "".join(str(int(x) ^ int(y)) for x, y in zip(a, b))
ham = lambda a, b: sum(x != y for x, y in zip(a, b))
V, TRI = "101", ["101", "111", "010"]   # stand on TRI[0]; XOR with V carries it to 000
def symmetry_panel(dx, corners, you_label, title):
    ox, oy = CX + dx, CY
    pt = {k: (x + dx, y) for k, (x, y, _) in P.items()}
    out = [f'<text x="{ox}" y="24" text-anchor="middle" class="t-title">{title}</text>',
           f'<circle cx="{ox}" cy="{oy}" r="{f(R)}" class="f-sphere s-line" stroke-width="1.2"/>']
    for a, b in edges:
        dash = ' stroke-dasharray="4 4"' if HIDDEN in (a, b) else ""
        out.append(f'<line x1="{f(pt[a][0])}" y1="{f(pt[a][1])}" x2="{f(pt[b][0])}" y2="{f(pt[b][1])}" class="s-line" stroke-width="1.2"{dash}/>')
    for k, (x, y) in pt.items():
        if k not in corners:
            cls = "f-ground s-line" if k == HIDDEN else "f-muted"
            out.append(f'<circle cx="{f(x)}" cy="{f(y)}" r="3.5" class="{cls}"/>')
    tri = [pt[k] for k in corners]
    out.append('<polygon points="' + " ".join(f"{f(x)},{f(y)}" for x, y in tri)
               + '" class="f-acc-soft s-acc" stroke-width="2.2" stroke-linejoin="round"/>')
    gx, gy = sum(p[0] for p in tri) / 3, sum(p[1] for p in tri) / 3
    for i in range(3):
        a, b = corners[i], corners[(i + 1) % 3]
        mx, my = (pt[a][0] + pt[b][0]) / 2, (pt[a][1] + pt[b][1]) / 2
        n = math.hypot(mx - gx, my - gy) or 1
        lx, ly = mx + 12 * (mx - gx) / n, my + 12 * (my - gy) / n
        out += [f'<circle cx="{f(lx)}" cy="{f(ly)}" r="9.5" class="f-ground"/>',
                f'<text x="{f(lx)}" y="{f(ly + 4)}" text-anchor="middle" class="t-acc t-strong">{ham(a, b)}</text>']
    for j, k in enumerate(corners):
        x, y = pt[k]
        ux, uy = x - ox, y - oy; n = math.hypot(ux, uy) or 1
        lx, ly = x + 16 * ux / n, y + 16 * uy / n
        anchor = "start" if ux > 8 else "end" if ux < -8 else "middle"
        out += [f'<circle cx="{f(x)}" cy="{f(y)}" r="6" class="f-acc"/>',
                f'<text x="{f(lx)}" y="{f(ly + 4)}" text-anchor="{anchor}" class="t-acc">{k}</text>']
        if j == 0:
            out += [f'<circle cx="{f(x)}" cy="{f(y)}" r="11" class="s-acc" stroke-width="1.6"/>',
                    f'<text x="{f(lx)}" y="{f(ly + 24)}" text-anchor="{anchor}" class="t-muted">{you_label}</text>']
    dists = ", ".join(str(d) for d in sorted(ham(corners[i], corners[(i + 1) % 3]) for i in range(3)))
    out.append(f'<text x="{ox}" y="336" text-anchor="middle" class="t-muted">distances {dists}</text>')
    return out
moved = [xor(k, V) for k in TRI]
fig6 = symmetry_panel(0, TRI, "you stand here", "before")
fig6 += symmetry_panel(460, moved, f"you, now at {moved[0]}", f"after XOR with {V}")
fig6 += [f'<line x1="378" y1="{CY}" x2="540" y2="{CY}" class="s-ink" stroke-width="1.6" marker-end="url(#f6-arr)"/>',
         f'<text x="460" y="{CY - 26}" text-anchor="middle" class="t-sans t-strong">XOR every corner</text>',
         f'<text x="460" y="{CY - 10}" text-anchor="middle" class="t-sans">with {V}</text>',
         f'<text x="460" y="{CY + 24}" text-anchor="middle" class="t-muted">d(x⊕v, y⊕v) = d(x, y)</text>']

# ---------- figures 7 and 8: the cube seen straight down its long diagonal ----------
TOP = "001"   # which one-bit corner points straight up
pop = lambda k: k.count("1")
rot3 = lambda k: k[-1] + k[:-1]                                   # permute by 1, for 3 bits
comp = lambda k: "".join("1" if c == "0" else "0" for c in k)     # XOR with 111
LAYER1 = [TOP, rot3(TOP), rot3(rot3(TOP))]
LAYER2 = [comp(k) for k in LAYER1]
def diag_view(cx, cy, scale):
    """Exact orthographic view down the 000 -> 111 axis, turned so TOP points up."""
    e1 = (1 / math.sqrt(2), -1 / math.sqrt(2), 0.0)
    e2 = (1 / math.sqrt(6), 1 / math.sqrt(6), -2 / math.sqrt(6))
    raw = {bits(c): (sum(a * b for a, b in zip(c, e1)), sum(a * b for a, b in zip(c, e2)))
           for c in itertools.product([-1, 1], repeat=3)}
    turn = math.pi / 2 - math.atan2(raw[TOP][1], raw[TOP][0])
    return {k: (cx + scale * (u * math.cos(turn) - v * math.sin(turn)),
                cy - scale * (u * math.sin(turn) + v * math.cos(turn))) for k, (u, v) in raw.items()}
def star_view(cx, cy, scale, gap, small=False, faint_layer2=False):
    pt = diag_view(cx, cy, scale)
    out = [f'<circle cx="{cx}" cy="{cy}" r="{f(scale * math.sqrt(3))}" class="f-sphere s-line" stroke-width="1.2"/>']
    for a, b in edges:
        dash = ' stroke-dasharray="4 4"' if "111" in (a, b) else ""
        out.append(f'<line x1="{f(pt[a][0])}" y1="{f(pt[a][1])}" x2="{f(pt[b][0])}" y2="{f(pt[b][1])}" class="s-ink" stroke-width="1.2"{dash}/>')
    for layer, cls in ((LAYER2, "s-warm"), (LAYER1, "s-acc")):
        w = 1.4 if (faint_layer2 and layer is LAYER2) else 2.6
        out.append('<polygon points="' + " ".join(f"{f(pt[k][0])},{f(pt[k][1])}" for k in layer)
                   + f'" class="{cls}" stroke-width="{w}" stroke-linejoin="round"/>')
    for k in LAYER1 + LAYER2:
        x, y = pt[k]
        ux, uy = x - cx, y - cy; n = math.hypot(ux, uy)
        lx, ly = x + gap * ux / n, y + gap * uy / n
        anchor = "start" if ux > 8 else "end" if ux < -8 else "middle"
        one = pop(k) == 1
        out += [f'<circle cx="{f(x)}" cy="{f(y)}" r="{4.5 if small else 5.5}" class="{"f-acc" if one else "f-warm"}"/>',
                f'<text x="{f(lx)}" y="{f(ly + 4)}" text-anchor="{anchor}" class="{"t-acc" if one else "t-warm"}{" t-small" if small else ""}">{k}</text>']
    return out, pt
fig7, _ = star_view(230, 200, 150 / math.sqrt(3), 18)
fig7 += ['<rect x="192" y="189" width="76" height="22" rx="11" class="f-ground s-line"/>',
         '<text x="230" y="204" text-anchor="middle">000 / 111</text>']

def arc(cx, cy, r, a0, a1, sweep, mid):
    x0, y0 = cx + r * math.cos(a0), cy + r * math.sin(a0)
    x1, y1 = cx + r * math.cos(a1), cy + r * math.sin(a1)
    return f'<path d="M{f(x0)},{f(y0)} A{r},{r} 0 0,{sweep} {f(x1)},{f(y1)}" class="s-acc" stroke-width="2" marker-end="url(#{mid})"/>'
fig8 = []
panels = [("permute", "shift the bits by 1", "= turn the star 120°",
           [f"{k} → {rot3(k)}" for k in LAYER1], "each triangle turns into itself"),
          ("XOR 111", "flip every bit", "= turn the star 180°",
           [f"{k} ↔ {comp(k)}" for k in LAYER1] + ["000 ↔ 111"], "the two triangles swap")]
for i, (title, l1, l2, maps, note) in enumerate(panels):
    x0 = 10 + i * 455
    fig8 += [f'<rect x="{x0}" y="10" width="445" height="250" rx="14" class="f-panel"/>',
             f'<text x="{x0 + 24}" y="44" class="t-title">{title}</text>',
             f'<text x="{x0 + 24}" y="72" class="t-sans">{l1}</text>',
             f'<text x="{x0 + 24}" y="90" class="t-sans">{l2}</text>']
    fig8 += [f'<text x="{x0 + 24}" y="{128 + 18 * j}" class="t-acc">{m}</text>' for j, m in enumerate(maps)]
    fig8.append(f'<text x="{x0 + 24}" y="236" class="t-muted t-small">{note}</text>')
    scx, scy = x0 + 320, 135
    body, pt = star_view(scx, scy, 70 / math.sqrt(3), 12, small=True, faint_layer2=(i == 0))
    fig8 += body + [f'<circle cx="{scx}" cy="{scy}" r="3" class="f-ink"/>']
    if i == 0:
        ang = lambda k: math.atan2(pt[k][1] - scy, pt[k][0] - scx)
        for k in LAYER1:
            a0, a1 = ang(k), ang(rot3(k))
            d = (a1 - a0 + math.pi) % (2 * math.pi) - math.pi
            sgn, gap = (1 if d > 0 else -1), math.radians(18)
            fig8.append(arc(scx, scy, 112, a0 + sgn * gap, a1 - sgn * gap, 1 if sgn > 0 else 0, "f8-arr-acc"))
    else:
        fig8.append(arc(scx, scy, 112, math.radians(185), math.radians(355), 1, "f8-arr-acc"))

# ---------- assembly ----------
def marker(mid, cls):
    return (f'<marker id="{mid}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" '
            f'orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" class="{cls}"/></marker>')

def svg(vb, label, body, defs=""):
    return (f'<svg viewBox="{vb}" role="img" aria-label="{label}">'
            + (f"<defs>{defs}</defs>" if defs else "") + "".join(body) + "</svg>")

FIGURES = {
    "{{SVG1}}": svg("0 0 460 330", "A three-bit cube with all eight corners on one sphere; the path from 001 through 011 to 111 walks two edges.", fig1),
    "{{SVG2}}": svg("0 0 920 250", f"Distance distribution drawn to scale from 0 to {DIM} bits: normal readings spike near {NORMAL_MEAN:.0f}, everything unrelated spikes at {UNRELATED_MEAN:.0f} plus or minus {UNRELATED_SD:.0f}.", fig2),
    "{{SVG3}}": svg("0 0 920 390", "Schematic map: a normal huddle with its bundle and a dashed RejectNet around it, a fan-on huddle, a smoke failure huddle, and an unknown reading rejected at distance 2043.", fig3, marker("f3-arr", "f-ink")),
    "{{SVG4}}": svg("0 0 920 230", "Three operations: bundle makes a point close to all inputs, bind makes a point far from both inputs, permute rotates to mark order.", fig4, marker("f4-arr", "f-ink") + marker("f4-arr-acc", "f-acc")),
    "{{SVG5}}": svg("0 0 920 340", "Host row: text, embedding model, SimHash, hypervector. Device row: MQ-2 sensor, features, codebook, RejectNet, store. Only 512-byte hypervectors cross from host to device.", fig5, marker("f5-arr", "f-ink") + marker("f5-arr-acc", "f-acc")),
    "{{SVG6}}": svg("0 0 920 350", f"Two 3-bit cubes: XOR with {V} moves the triangle {', '.join(TRI)} to {', '.join(moved)}, and its three distances stay the same.", fig6, marker("f6-arr", "f-ink")),
    "{{SVG7}}": svg("0 0 460 400", "The 3-bit cube seen down its long diagonal: a hexagon whose corners form two triangles, the one-bit corners in teal and the two-bit corners in amber, with 000 and 111 overlapping in the centre.", fig7),
    "{{SVG8}}": svg("0 0 920 270", f"Two panels: permute turns the star 120 degrees, mapping {', '.join(m for m in panels[0][3])}; XOR with 111 turns it 180 degrees, swapping each corner with its opposite.", fig8, marker("f8-arr-acc", "f-acc")),
}

GOOGLE_FONTS = re.compile(r'<link rel="stylesheet" href="(https://fonts\.googleapis\.com/[^"]+)">')
FONT_NOTICE = ("/* Atkinson Hyperlegible (Braille Institute of America), Bricolage Grotesque (The Bricolage\n"
               "   Grotesque Project Authors), JetBrains Mono (The JetBrains Mono Project Authors).\n"
               "   Latin subsets, embedded as data URIs. All under the SIL Open Font License 1.1:\n"
               "   https://openfontlicense.org */\n")

def fetch_fonts(fragment):
    """Download the latin subset of each typeface and store it as data URIs in fonts.css."""
    url = GOOGLE_FONTS.search(fragment).group(1).replace("&amp;", "&")
    ua = {"User-Agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/126.0 Safari/537.36"}
    css = urllib.request.urlopen(urllib.request.Request(url, headers=ua), timeout=20).read().decode()
    cache, blocks = {}, []
    for subset, block in re.findall(r"/\* (\S+) \*/\s*(@font-face\s*\{.*?\})", css, re.S):
        if subset != "latin":
            continue
        src = re.search(r"url\((https://[^)]+)\)", block).group(1)
        if src not in cache:
            data = urllib.request.urlopen(urllib.request.Request(src, headers=ua), timeout=20).read()
            cache[src] = "data:font/woff2;base64," + base64.b64encode(data).decode()
        blocks.append(block.replace(src, cache[src]))
    FONTS.write_text(FONT_NOTICE + "\n".join(blocks) + "\n")
    print(f"fetched {len(blocks)} @font-face rules into {FONTS}")

def standalone(fragment):
    """Wrap the fragment as a complete page with embedded fonts, so it opens with no network."""
    body = re.sub(r'<link rel="preconnect"[^>]*>\s*', "", fragment)
    fonts = FONTS.read_text().rstrip("\n") if FONTS.exists() else ""
    body = GOOGLE_FONTS.sub(lambda _: "<style>\n" + fonts + "\n</style>" if fonts else "", body)
    title = re.search(r"<title>.*?</title>", body).group(0)
    body = body.replace(title, "", 1)
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
{title}
<style>body {{ margin: 0; }} img {{ max-width: 100%; }}</style>
</head>
<body>
{body}
</body>
</html>
"""

def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--fragment", type=pathlib.Path, help="also write an Artifact-ready fragment here")
    ap.add_argument("--fetch-fonts", action="store_true", help="re-download fonts.css (needs network)")
    args = ap.parse_args()

    fragment = TEMPLATE.read_text()
    for key, figure in FIGURES.items():
        fragment = fragment.replace(key, figure)
    if args.fetch_fonts:
        fetch_fonts(fragment)
    OUT.write_text(standalone(fragment), encoding="utf-8")
    print(f"wrote {OUT} ({OUT.stat().st_size // 1024} KB), DIM = {DIM}")
    if args.fragment:
        args.fragment.write_text(fragment, encoding="utf-8")
        print(f"wrote fragment {args.fragment}")

if __name__ == "__main__":
    main()
