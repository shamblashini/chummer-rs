#!/usr/bin/env python3
"""Build the three print-sheet design mockups from a chummer-rs print XML.

Usage:
    chummer-cli sheet Munin_Career.chum5 --xml munin-print.xml -o current-raw.html
    python3 generate.py munin-print.xml [--out DIR] [--current-in current-raw.html] [--render]

Writes design-1-clean.html, design-2-dense.html and design-3-flavour.html into
DIR (default: this script's folder). With --current-in, it also writes
current.html: the given default-sheet output with its full-size portrait
replaced by a ~300 px JPEG (same layout, much smaller file). With --render, it
prints every HTML file to a US Letter PDF with Playwright's chrome-headless-shell
(under nice -n 19) and writes png/<name>-N.png at 90 dpi with pdftoppm.

Only the Python standard library is required; Pillow is optional (without it
the designs leave the portrait out).
"""

import argparse
import base64
import glob
import html
import io
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
DOSSIER_FILE_ID = "ED1E45"  # file number shown in design 3; not derived from the data


# --------------------------------------------------------------------------
# Data extraction
# --------------------------------------------------------------------------

def t(e, tag, default=""):
    if e is None:
        return default
    v = e.findtext(tag)
    return default if v is None else v.strip()


def b(e, tag):
    return t(e, tag) == "True"


def num(s, default=0):
    m = re.search(r"-?\d+(?:\.\d+)?", s or "")
    if not m:
        return default
    v = float(m.group(0))
    return int(v) if v.is_integer() else v


def container(ch, tag):
    """First element with this tag that has children (the XML repeats some
    tags as plain scalars, e.g. <attributes>0</attributes>)."""
    for e in ch.findall(tag):
        if len(e):
            return e
    return None


def kids(ch, tag, child):
    c = container(ch, tag)
    return [] if c is None else [x for x in c if x.tag == child]


def portrait_uri(ch, grey=False, size=300):
    raw = t(ch, "mainmugshotbase64")
    if not raw:
        return None
    try:
        from PIL import Image
    except ImportError:
        return None
    im = Image.open(io.BytesIO(base64.b64decode(raw)))
    if im.mode in ("RGBA", "LA", "P"):
        im = im.convert("RGBA")
        bg = Image.new("RGBA", im.size, (255, 255, 255, 255))
        bg.alpha_composite(im)
        im = bg
    im = im.convert("L" if grey else "RGB")
    im.thumbnail((size, size))
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=78, optimize=True)
    return "data:image/jpeg;base64," + base64.b64encode(buf.getvalue()).decode()


def gear_tree(parent):
    out = []
    for g in parent:
        if g.tag != "gear":
            continue
        out.append({
            "name": t(g, "fullname") or t(g, "name"),
            "category": t(g, "category"),
            "rating": num(t(g, "rating")),
            "qty": t(g, "qty"),
            "extra": t(g, "extra"),
            "iscommlink": b(g, "iscommlink"),
            "dr": t(g, "devicerating"), "a": t(g, "attack"), "s": t(g, "sleaze"),
            "d": t(g, "dataprocessing"), "f": t(g, "firewall"),
            "cost": t(g, "cost"),
            "children": gear_tree(g.find("children") if g.find("children") is not None else []),
        })
    return out


def weapon(w):
    r = w.find("ranges")
    rng = []
    if r is not None:
        for k, mod in (("short", "0"), ("medium", "−1"), ("long", "−3"), ("extreme", "−6")):
            v = t(r, k)
            if v:
                rng.append((k[0].upper(), v, mod))
    ub = []
    u = w.find("underbarrel")
    if u is not None:
        ub = [weapon(x) for x in u if x.tag == "weapon"]
    return {
        "name": t(w, "fullname") or t(w, "name"), "category": t(w, "category"),
        "type": t(w, "type"), "reach": t(w, "reach"), "acc": t(w, "accuracy"),
        "dv": t(w, "damage"), "ap": t(w, "ap"), "mode": t(w, "mode") or "—",
        "rc": t(w, "rc"), "ammo": t(w, "ammo"), "pool": t(w, "dicepool"),
        "skill": t(w, "skill"), "ranges": rng, "underbarrel": ub,
        "conceal": t(w, "conceal"),
    }


def load(path):
    ch = ET.parse(path).getroot().find("character")
    d = {}
    d["alias"] = t(ch, "alias") or t(ch, "name")
    d["name"] = t(ch, "name")
    mv = t(ch, "metavariant")
    d["metatype"] = t(ch, "metatype") + (f" ({mv})" if mv else "")
    for k in ("gender", "age", "eyes", "hair", "skin", "height", "weight",
              "playername", "buildmethod", "karma", "totalkarma", "nuyen",
              "totaless", "movement", "description", "background", "notes",
              "init", "astralinit", "matrixarinit", "matrixcoldinit",
              "matrixhotinit", "riggerinit", "surprise", "composure",
              "judgeintentions", "judgeintentionsresist", "memory",
              "liftandcarry", "liftweight", "carryweight", "dodge", "armor",
              "armordicephysical", "armordicestun", "physicalcm", "stuncm",
              "cmthreshold", "physicalcmthresholdoffset", "stuncmthresholdoffset",
              "cmoverflow", "physicalcmnaturalrecovery", "stuncmnaturalrecovery",
              "limitphysical", "limitmental", "limitsocial", "limitastral",
              "totalstreetcred", "totalnotoriety", "totalpublicawareness",
              "edgeremaining", "initiategrade", "spelllimit",
              "fatigueresist", "radiationresist", "sonicresist",
              "toxincontactresist", "toxiningestionresist", "toxininhalationresist",
              "toxininjectionresist", "pathogencontactresist", "pathogeningestionresist",
              "pathogeninhalationresist", "pathogeninjectionresist",
              "physiologicaladdictionresistfirsttime", "psychologicaladdictionresistfirsttime",
              "directmanaresist", "directphysicalresist", "indirectdefenseresist",
              "detectionspellresist", "illusionmanaresist", "illusionphysicalresist",
              "manipulationmentalresist", "manipulationphysicalresist",
              "decreasebodresist", "decreaseagiresist", "decreaserearesist",
              "decreasestrresist", "decreasecharesist", "decreaseintresist",
              "decreaselogresist", "decreasewilresist"):
        d[k] = t(ch, k)
    d["career"] = b(ch, "created")
    d["magician"] = b(ch, "magician")
    d["adept"] = b(ch, "adept")

    d["attrs"] = {}
    for a in kids(ch, "attributes", "attribute"):
        d["attrs"][t(a, "name_english")] = {
            "total": t(a, "total"), "base": t(a, "base"), "min": t(a, "min"),
            "max": t(a, "max"), "aug": t(a, "aug")}
    d["mag"] = num(d["attrs"].get("MAG", {}).get("total", "0"))
    d["edge"] = num(d["attrs"].get("EDG", {}).get("total", "0"))

    d["limitmods"] = []
    for tag, lim in (("limitmodifiersphys", "Physical"), ("limitmodifiersment", "Mental"),
                     ("limitmodifierssoc", "Social")):
        for m in kids(ch, tag, "limitmodifier"):
            d["limitmods"].append((lim, t(m, "name"), t(m, "bonus"), t(m, "condition")))

    groups = {}
    skills, know = [], []
    sk = container(ch, "skills")
    for x in sk if sk is not None else []:
        if x.tag == "skillgroup":
            groups[t(x, "name")] = t(x, "rating")
            continue
        if x.tag != "skill":
            continue
        specs = [t(s, "name") for s in x.iter("skillspecialization")]
        s = {"name": t(x, "name"), "group": t(x, "skillgroup"),
             "category": t(x, "skillcategory"), "rating": num(t(x, "rating")),
             "pool": t(x, "total"), "attr": t(x, "displayattribute"),
             "specs": specs, "lang": b(x, "islanguage"),
             # The print XML flags no language as native; a language at
             # rating 0 is the native one (Chummer prints it as "N").
             "native": b(x, "islanguage") and num(t(x, "rating")) == 0}
        if s["group"] == "None":
            s["group"] = ""
        if b(x, "knowledge"):
            know.append(s)
        elif s["rating"] > 0:
            skills.append(s)
    d["skills"] = sorted(skills, key=lambda s: s["name"])
    d["knowledge"] = sorted(know, key=lambda s: (s["lang"], s["name"]))
    d["groups"] = {g: r for g, r in groups.items() if num(r) > 0}
    d["skillpool"] = {s["name"]: s["pool"] for s in skills}

    def unparen(x):
        x = x.strip()
        return x[1:-1].strip() if x.startswith("(") and x.endswith(")") else x
    d["qualities"] = [{"name": t(q, "name"), "extra": unparen(t(q, "extra")),
                       "bp": t(q, "bp"), "type": t(q, "qualitytype"),
                       "source": t(q, "qualitysource")}
                      for q in kids(ch, "qualities", "quality")]
    sinq = [q for q in d["qualities"] if q["name"].startswith("SINner")]
    d["sin"] = (f"{sinq[0]['name']} — {sinq[0]['extra']}" if sinq else "SINless")

    d["weapons"] = [weapon(w) for w in kids(ch, "weapons", "weapon")]
    d["armors"] = [{"name": t(a, "fullname") or t(a, "name"), "armor": t(a, "armor"),
                    "equipped": b(a, "equipped"),
                    "mods": [t(m, "name") for m in a.iter("armormod")]}
                   for a in kids(ch, "armors", "armor")]
    d["ware"] = [{"name": t(c, "fullname") or t(c, "name"), "ess": t(c, "ess"),
                  "grade": t(c, "grade"), "category": t(c, "category"),
                  "rating": num(t(c, "rating")), "location": t(c, "location")}
                 for c in kids(ch, "cyberwares", "cyberware")]
    g = container(ch, "gears")
    d["gear"] = gear_tree(g if g is not None else [])

    d["spells"] = [{"name": t(s, "name") + (f" ({t(s, 'extra')})" if t(s, "extra") else ""),
                    "category": t(s, "category"), "type": t(s, "type"),
                    "range": t(s, "range"), "damage": t(s, "damage"),
                    "duration": t(s, "duration"), "dv": t(s, "dv"),
                    "pool": t(s, "dicepool"), "descriptors": t(s, "descriptors"),
                    "source": f"{t(s, 'source')} {t(s, 'page')}"}
                   for s in kids(ch, "spells", "spell")]
    tr = container(ch, "tradition")
    d["tradition"] = {k: t(tr, k) for k in (
        "name", "drainattributes", "drainvalue", "spiritcombat", "spiritdetection",
        "spirithealth", "spiritillusion", "spiritmanipulation", "spiritform")}
    ms = kids(ch, "mentorspirits", "mentorspirit")
    d["mentor"] = ({"name": t(ms[0], "name"), "adv": t(ms[0], "advantage"),
                    "dis": t(ms[0], "disadvantage")} if ms else None)
    d["metamagics"] = [t(m, "name") for m in kids(ch, "metamagics", "metamagic")]
    d["arts"] = [t(m, "name") for m in kids(ch, "arts", "art")]

    d["contacts"] = [{"name": t(c, "name"), "role": t(c, "role"), "location": t(c, "location"),
                      "c": t(c, "connection"), "l": t(c, "loyalty"), "type": t(c, "type")}
                     for c in kids(ch, "contacts", "contact")]
    handler = sorted(d["contacts"], key=lambda c: ("fixer" not in c["role"].lower(), -num(c["c"])))
    d["handler"] = handler[0] if handler else None
    d["lifestyles"] = [{"name": t(l, "name"), "base": t(l, "baselifestyle"),
                        "monthly": t(l, "totalmonthlycost"), "months": t(l, "months"),
                        "increment": t(l, "increment")}
                       for l in kids(ch, "lifestyles", "lifestyle")]
    d["vehicles"] = []
    for v in kids(ch, "vehicles", "vehicle"):
        gv = v.find("gears")
        d["vehicles"].append({
            "name": t(v, "fullname") or t(v, "name"), "category": t(v, "category"),
            "drone": b(v, "isdrone"),
            "stats": [(k, t(v, k.lower())) for k in
                      ("Handling", "Accel", "Speed", "Pilot", "Body", "Armor", "Sensor")],
            "cm": t(v, "physicalcm"), "dr": t(v, "devicerating"),
            "gear": gear_tree(gv if gv is not None else [])})
    d["portrait"] = portrait_uri(ch)
    d["portrait_px"] = None
    try:
        from PIL import Image
        d["portrait_px"] = Image.open(io.BytesIO(base64.b64decode(t(ch, "mainmugshotbase64")))).size[0]
    except Exception:
        pass
    d["portrait_grey"] = portrait_uri(ch, grey=True)
    return d


# --------------------------------------------------------------------------
# Shared helpers
# --------------------------------------------------------------------------

E = html.escape


def drain_at(dv, force):
    """Drain value at a Force, or None when the spell's DV is not F±n."""
    m = re.fullmatch(r"\s*F\s*([+-]\s*\d+)?\s*", dv or "")
    if not m:
        return None
    mod = int(m.group(1).replace(" ", "")) if m.group(1) else 0
    return max(2, force + mod)


def cm_marks(boxes, threshold, offset, last):
    marks = {}
    for i in range(1, boxes + 1):
        k = i - offset
        if k > 0 and k % threshold == 0:
            marks[i] = f"−{k // threshold}"
    marks[boxes] = last
    return marks


def profile(d):
    bits = []
    if d["magician"]:
        bits.append(f"{d['tradition']['name']} magician".strip())
    elif d["adept"]:
        bits.append("Adept")
    if num(d["initiategrade"]):
        bits.append(f"Initiate grade {d['initiategrade']}")
    if d["mentor"]:
        bits.append(f"Mentor: {d['mentor']['name'].replace(' (Alt)', '')}")
    bits.append("Career" if d["career"] else "Creation")
    bits.append({"SumtoTen": "Sum-to-Ten"}.get(d["buildmethod"], d["buildmethod"]))
    return " · ".join(bits)


def ammo_count(s):
    m = re.match(r"\s*(\d+)", s or "")
    return int(m.group(1)) if m else 0


def range_text(w, sep=" · "):
    if w["ranges"]:
        return sep.join(f"{k} {v} ({m})" for k, v, m in w["ranges"])
    return f"melee · reach {w['reach']}"


def flat_gear(items, depth=0):
    for g in items:
        yield depth, g
        yield from flat_gear(g["children"], depth + 1)


def gear_label(g):
    s = g["name"]
    if g["rating"] and "Rating" not in s:
        s += f" R{g['rating']}"
    if g["qty"] not in ("", "1"):
        s += f" ×{g['qty']}"
    return s


def commlink_stats(g):
    return f"DR {g['dr']} · A {g['a']} S {g['s']} D {g['d']} F {g['f']}"


ATTR_ORDER = ["BOD", "AGI", "REA", "STR", "WIL", "LOG", "INT", "CHA", "EDG", "MAG", "ESS"]


def attr_total(d, k):
    return d["attrs"].get(k, {}).get("total", "")


def spell_groups(d):
    out = {}
    for s in d["spells"]:
        out.setdefault(s["category"], []).append(s)
    return sorted(out.items())


RESIST_ROWS = [
    ("Combat spells", [("Direct mana", "directmanaresist"), ("Direct physical", "directphysicalresist"),
                       ("Indirect (defense)", "indirectdefenseresist")]),
    ("Detection / illusion", [("Detection", "detectionspellresist"), ("Illusion mana", "illusionmanaresist"),
                              ("Illusion physical", "illusionphysicalresist")]),
    ("Manipulation", [("Mental", "manipulationmentalresist"), ("Physical", "manipulationphysicalresist")]),
    ("Decrease attribute", [("BOD", "decreasebodresist"), ("AGI", "decreaseagiresist"),
                            ("REA", "decreaserearesist"), ("STR", "decreasestrresist"),
                            ("CHA", "decreasecharesist"), ("INT", "decreaseintresist"),
                            ("LOG", "decreaselogresist"), ("WIL", "decreasewilresist")]),
    ("Toxins (contact/ingest/inhale/inject)", [("", "toxincontactresist"), ("", "toxiningestionresist"),
                                                ("", "toxininhalationresist"), ("", "toxininjectionresist")]),
    ("Pathogens (contact/ingest/inhale/inject)", [("", "pathogencontactresist"), ("", "pathogeningestionresist"),
                                                   ("", "pathogeninhalationresist"), ("", "pathogeninjectionresist")]),
    ("Other", [("Fatigue", "fatigueresist"), ("Radiation", "radiationresist"), ("Sonic", "sonicresist"),
               ("Addiction phys.", "physiologicaladdictionresistfirsttime"),
               ("Addiction psych.", "psychologicaladdictionresistfirsttime")]),
]


def page_css(left, right_prefix, font, size="7.5pt", margin="11mm 11mm 13mm 11mm"):
    return f"""@page {{ size: letter; margin: {margin};
  @bottom-left {{ content: "{left}"; font: {size} {font}; color: #000; }}
  @bottom-right {{ content: "{right_prefix}page " counter(page) " / " counter(pages); font: {size} {font}; color: #000; }} }}"""


def doc(title, css, body):
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{E(title)}</title>
<style>
*{{box-sizing:border-box}}
html{{-webkit-print-color-adjust:exact;print-color-adjust:exact}}
body{{margin:0;background:#fff;color:#000}}
@media screen{{body{{max-width:8.5in;margin:0 auto;padding:12mm 11mm;}}}}
{css}
</style></head><body>
{body}
</body></html>
"""


# --------------------------------------------------------------------------
# Design 1: clean modern
# --------------------------------------------------------------------------

def design1(d):
    font = "system-ui, 'Inter', 'Noto Sans', 'Liberation Sans', Arial, sans-serif"
    css = page_css(f"{d['alias']} — {d['name']}", "Chummer-rs · ", font) + f"""
body{{font:8.6pt/1.35 {font}}}
h1{{font-size:24pt;line-height:1;margin:0;font-weight:750;letter-spacing:-.01em}}
h2{{font-size:9pt;text-transform:uppercase;letter-spacing:.08em;margin:0 0 4px;font-weight:700}}
h3{{font-size:8.6pt;margin:4px 0 2px;font-weight:700}}
.sub{{font-size:10pt;margin-top:3px}}
.muted{{color:#555}}
.small{{font-size:7.4pt}}
.card{{border:.6pt solid #888;border-radius:6px;padding:6px 8px;margin:0 0 7px;break-inside:avoid}}
.page{{break-before:page}}
header{{display:grid;grid-template-columns:22mm 1fr auto;gap:10px;align-items:start;margin-bottom:8px;
  padding-bottom:7px;border-bottom:1.2pt solid #000}}
header img{{width:22mm;height:28mm;object-fit:cover;border-radius:4px;border:.6pt solid #888}}
.stats{{display:grid;grid-template-columns:repeat(3,auto);gap:2px 12px;border:.6pt solid #888;border-radius:6px;padding:5px 8px}}
.stats div{{text-align:right}} .stats b{{display:block;font-size:12pt;line-height:1.1}}
.stats span{{font-size:6.6pt;text-transform:uppercase;letter-spacing:.06em;color:#444}}
.grid2{{display:grid;grid-template-columns:1fr 1fr;gap:0 8px}}
.grid-6040{{display:grid;grid-template-columns:47% 53%;gap:0 8px}}
.attrs{{display:grid;grid-template-columns:repeat(4,1fr);gap:4px}}
.attr{{border:.6pt solid #aaa;border-radius:4px;text-align:center;padding:2px 0 3px}}
.attr .l{{font-size:6.8pt;letter-spacing:.08em;font-weight:700}}
.attr .v{{font-size:16pt;font-weight:700;line-height:1.05}}
.attr .m{{font-size:6.4pt;color:#555}}
.lims{{display:grid;grid-template-columns:repeat(4,1fr);gap:4px;text-align:center}}
.lims .v{{font-size:16pt;font-weight:700;line-height:1.1}}
.lims .l{{font-size:6.8pt;letter-spacing:.06em;text-transform:uppercase}}
dl.kv{{display:grid;grid-template-columns:auto 1fr;gap:1px 8px;margin:0}}
dl.kv dt{{color:#333}} dl.kv dd{{margin:0;text-align:right;font-weight:600}}
table{{border-collapse:collapse;width:100%}}
th{{font-size:6.6pt;text-transform:uppercase;letter-spacing:.06em;text-align:left;font-weight:700;
  border-bottom:.8pt solid #000;padding:1px 3px}}
td{{padding:1.4px 3px;border-bottom:.4pt solid #ccc;vertical-align:top}}
td.n,th.n{{text-align:center}} td.pool{{font-weight:800;text-align:center;font-size:9.4pt}}
.cm{{display:grid;gap:2px;margin:2px 0 4px}}
.cm div{{border:.8pt solid #000;height:15px;font-size:6.6pt;text-align:center;line-height:14px;border-radius:2px}}
.cm div.ov{{border-style:dashed;border-color:#666;color:#444}}
.edge span{{display:inline-block;width:13px;height:13px;border:.9pt solid #000;border-radius:50%;margin-right:3px;vertical-align:middle}}
.pill{{display:inline-block;border:.6pt solid #888;border-radius:9px;padding:0 6px;margin:0 3px 2px 0;font-size:7pt}}
.weapon{{border-bottom:.4pt solid #ccc;padding:3px 0;break-inside:avoid}}
.weapon .top{{display:grid;grid-template-columns:1fr 42px 34px 50px 30px 34px 28px 48px;gap:4px;align-items:baseline}}
.weapon .top b.pool{{font-size:11pt;text-align:center}}
.weapon .rng{{font-size:7.2pt;color:#333;margin-top:1px}}
.weapon.ub{{margin-left:12px;border-bottom-style:dotted}}
.wh{{font-size:6.6pt;text-transform:uppercase;letter-spacing:.06em;font-weight:700;border-bottom:.8pt solid #000}}
.cols2{{columns:2;column-gap:14px}} .cols2>*{{break-inside:avoid}}
.tree{{margin:0;padding:0;list-style:none}}
.cols2>.gear{{break-inside:auto}} .tree li{{padding:1px 0;break-inside:avoid}} .tree .d1{{padding-left:12px}} .tree .d2{{padding-left:24px}}
.tree .d1::before,.tree .d2::before{{content:"› ";color:#777}}
.lines div{{border-bottom:.5pt solid #999;height:15px}}
.note p{{margin:0 0 4px}}
"""
    a = d["attrs"]
    cells = []
    for k in ATTR_ORDER[:8]:
        cells.append(f'<div class="attr"><div class="l">{k}</div><div class="v">{E(a[k]["total"])}</div>'
                     f'<div class="m">max {E(a[k]["max"])}</div></div>')
    for k in ("EDG", "MAG", "ESS"):
        cells.append(f'<div class="attr"><div class="l">{k}</div><div class="v">{E(a[k]["total"])}</div>'
                     f'<div class="m">max {E(a[k]["max"])}</div></div>')
    cells.append(f'<div class="attr"><div class="l">INIT</div><div class="v">{E(d["initvalue"] if "initvalue" in d else d["init"].split(" ")[0])}</div>'
                 f'<div class="m">+{E(d["init"].split("+")[-1].strip())}</div></div>')
    lm = "".join(f"<li>{E(lim)}: {E(n)} +{E(bn)} <span class='muted'>({E(c)})</span></li>"
                 for lim, n, bn, c in d["limitmods"])
    left = f"""<section class="card"><h2>Attributes</h2><div class="attrs">{''.join(cells)}</div></section>
<section class="card"><h2>Limits</h2><div class="lims">
<div><div class="v">{E(d['limitphysical'])}</div><div class="l">Physical</div></div>
<div><div class="v">{E(d['limitmental'])}</div><div class="l">Mental</div></div>
<div><div class="v">{E(d['limitsocial'])}</div><div class="l">Social</div></div>
<div><div class="v">{E(d['limitastral'])}</div><div class="l">Astral</div></div></div>
<ul class="small" style="margin:4px 0 0;padding-left:14px">{lm}</ul></section>"""

    der = [("Initiative", d["init"]), ("Astral initiative", d["astralinit"]),
           ("Matrix AR / cold / hot", f"{d['matrixarinit']} / {d['matrixcoldinit']} / {d['matrixhotinit']}"),
           ("Defense (REA+INT)", d["dodge"]), ("Damage resist (BOD+armor)", d["armordicephysical"]),
           ("Armor (worn)", d["armor"]), ("Surprise", d["surprise"]), ("Composure", d["composure"]),
           ("Judge intentions", d["judgeintentions"]), ("Memory", d["memory"]),
           ("Lift / carry", f"{d['liftandcarry']} dice · {d['liftweight']} / {d['carryweight']} kg"),
           ("Movement", d["movement"].split(",")[0]),
           ("Natural recovery", f"Phys {d['physicalcmnaturalrecovery']}/day · Stun {d['stuncmnaturalrecovery']}/hr")]
    derived = '<section class="card"><h2>Derived</h2><dl class="kv">' + "".join(
        f"<dt>{E(k)}</dt><dd>{E(v)}</dd>" for k, v in der) + "</dl></section>"

    def grid(n, last, overflow=0, cols=3):
        th, off = num(d["cmthreshold"]), num(d[f"{'physical' if last == 'Down' else 'stun'}cmthresholdoffset"])
        m = cm_marks(n, th, off, last)
        cells = "".join(f"<div>{E(m.get(i, ''))}</div>" for i in range(1, n + 1))
        cells += "".join(f'<div class="ov">{"Dead" if i == overflow else "+" + str(i)}</div>'
                         for i in range(1, overflow + 1))
        return f'<div class="cm" style="grid-template-columns:repeat({cols},1fr)">{cells}</div>'

    edge = "".join("<span></span>" for _ in range(d["edge"]))
    cond = f"""<section class="card"><h2>Condition</h2><div class="grid2">
<div><h3>Physical {E(d['physicalcm'])} + {E(d['cmoverflow'])}</h3>{grid(num(d['physicalcm']), 'Down', num(d['cmoverflow']))}</div>
<div><h3>Stun {E(d['stuncm'])}</h3>{grid(num(d['stuncm']), 'KO')}</div></div>
<div class="edge"><b>Edge</b> {edge} <span class="small muted" style="border:0;width:auto;height:auto">{E(d['edgeremaining'])} of {d['edge']} left</span></div></section>"""

    rows = d["skills"]
    half = (len(rows) + 1) // 2

    def sktable(rs):
        body = "".join(
            f"<tr><td>{E(s['name'])}{(' <span class=small>(' + E(', '.join(s['specs'])) + ')</span>') if s['specs'] else ''}"
            f"{(' <span class=\"muted small\">· ' + E(s['group']) + '</span>') if s['group'] else ''}</td>"
            f"<td class=n>{E(s['attr'])}</td><td class=n>{s['rating']}</td><td class=pool>{E(s['pool'])}</td></tr>"
            for s in rs)
        return f"<table><tr><th>Skill</th><th class=n>Attr</th><th class=n>Rtg</th><th class=n>Pool</th></tr>{body}</table>"
    pills = "".join(f'<span class="pill">{E(g)} {E(r)}</span>' for g, r in sorted(d["groups"].items()))
    skills_html = f"""<section class="card"><h2>Active skills</h2><div style="margin-bottom:3px"><span class="small muted">Skill groups:</span> {pills}</div>
<div class="grid2">{sktable(rows[:half])}{sktable(rows[half:])}</div></section>"""

    hdr_img = f'<img src="{d["portrait"]}" alt="">' if d["portrait"] else "<div></div>"
    header = f"""<header>{hdr_img}<div><h1>{E(d['alias'])}</h1>
<div class="sub">{E(d['name'])} · {E(d['metatype'])} · {E(d['gender'])} · {E(d['age'])}</div>
<div class="small muted" style="margin-top:3px">{E(profile(d))} · Player: {E(d['playername'])}</div>
<div class="small muted">{E(d['height'])}, {E(d['weight'])} · eyes {E(d['eyes'])} · hair {E(d['hair'])} · skin {E(d['skin'])}</div></div>
<div class="stats">
<div><b>{E(d['karma'])}</b><span>Karma</span></div><div><b>{E(d['totalkarma'])}</b><span>Career karma</span></div><div><b>{E(d['nuyen'])}¥</b><span>Nuyen</span></div>
<div><b>{E(d['totaless'])}</b><span>Essence</span></div><div><b>{E(d['totalstreetcred'])}/{E(d['totalnotoriety'])}/{E(d['totalpublicawareness'])}</b><span>Cred/Noto/Aware</span></div><div><b>{E(d['edgeremaining'])}/{d['edge']}</b><span>Edge left</span></div>
</div></header>"""

    page1 = header + f'<div class="grid-6040"><div>{left}</div><div>{derived}{cond}</div></div>' + skills_html

    # page 2
    def kn_rows(ks):
        return "".join(
            f"<tr><td>{E(s['name'])}</td><td class=muted>{'Language' if s['lang'] else E(s['category'])}</td>"
            f"<td class=n>{E(s['attr'])}</td><td class=n>{'N' if s['native'] else s['rating']}</td>"
            f"<td class=pool>{'—' if s['native'] else E(s['pool'])}</td></tr>" for s in ks)
    knowledge = (f'<section class="card"><h2>Knowledge &amp; languages</h2><table><tr><th>Skill</th><th>Type</th>'
                 f'<th class=n>Attr</th><th class=n>Rtg</th><th class=n>Pool</th></tr>{kn_rows(d["knowledge"])}</table>'
                 f'<div class="small muted" style="margin-top:2px">N = native language</div></section>')

    def ql(kind):
        return "".join(f"<tr><td>{E(q['name'])}{(' — ' + E(q['extra'])) if q['extra'] else ''}"
                       f"{' <span class=\"muted small\">(metatype)</span>' if q['source'] == 'Metatype' else ''}</td>"
                       f"<td class=n>{E(q['bp'])}</td></tr>" for q in d["qualities"] if q["type"] == kind)
    quals = (f'<section class="card"><h2>Qualities</h2><div class="grid2">'
             f'<table><tr><th>Positive</th><th class=n>Karma</th></tr>{ql("Positive")}</table>'
             f'<table><tr><th>Negative</th><th class=n>Karma</th></tr>{ql("Negative")}</table></div></section>')

    def wrow(w, ub=False):
        return (f'<div class="weapon{" ub" if ub else ""}"><div class="top"><div><b>{"↳ " if ub else ""}{E(w["name"])}</b> '
                f'<span class="muted small">{E(w["category"])} · {E(w["skill"])}</span></div>'
                f'<b class="pool">{E(w["pool"])}</b><div class=n>{E(w["acc"])}</div><div><b>{E(w["dv"])}</b></div>'
                f'<div>{E(w["ap"])}</div><div>{E(w["mode"])}</div><div>{E(w["rc"])}</div><div>{E(w["ammo"]) if ammo_count(w["ammo"]) else "—"}</div></div>'
                f'<div class="rng">{E(range_text(w))}</div></div>'
                + "".join(wrow(u, True) for u in w["underbarrel"]))
    weapons = ('<section class="card"><h2>Weapons</h2><div class="weapon wh"><div class="top"><div>Weapon</div>'
               '<div style="text-align:center">Pool</div><div>Acc</div><div>DV</div><div>AP</div><div>Mode</div><div>RC</div><div>Ammo</div></div></div>'
               + "".join(wrow(w) for w in d["weapons"]) + "</section>")
    armor = ('<section class="card"><h2>Armor</h2><table><tr><th>Item</th><th class=n>Rating</th><th class=n>Worn</th></tr>'
             + "".join(f"<tr><td>{E(a['name'])}</td><td class=n>{E(a['armor'])}</td><td class=n>{'✓' if a['equipped'] else ''}</td></tr>"
                       for a in d["armors"])
             + f"<tr><td><b>Worn total</b></td><td class=n><b>{E(d['armor'])}</b></td><td></td></tr></table></section>")
    ess_used = round(6 - float(d["totaless"]), 2)
    ware = ('<section class="card"><h2>Cyberware &amp; bioware</h2><table><tr><th>Item</th><th>Grade</th><th class=n>Essence</th></tr>'
            + "".join(f"<tr><td>{E(w['name'])}{(' R' + str(w['rating'])) if w['rating'] else ''} <span class='muted small'>{E(w['category'])}</span></td>"
                      f"<td>{E(w['grade'])}</td><td class=n>{E(w['ess'])}</td></tr>" for w in d["ware"])
            + f"<tr><td><b>Essence</b> <span class=small>(6 − {ess_used:.2f})</span></td><td></td><td class=n><b>{E(d['totaless'])}</b></td></tr></table></section>")
    page2 = f'<div class="page">{weapons}<div class="grid2"><div>{armor}{ware}</div><div>{quals}</div></div>{knowledge}</div>'

    # page 3: magic
    tr = d["tradition"]
    trad = (f'<section class="card"><h2>Tradition — {E(tr["name"])}</h2><dl class="kv">'
            f'<dt>Drain</dt><dd>{E(tr["drainattributes"])} = {E(tr["drainvalue"])}</dd>'
            f'<dt>Spellcasting / Counterspelling</dt><dd>{E(d["skillpool"].get("Spellcasting", "—"))} / {E(d["skillpool"].get("Counterspelling", "—"))}</dd>'
            f'<dt>Summoning / Banishing / Binding</dt><dd>{E(d["skillpool"].get("Summoning", "—"))} / {E(d["skillpool"].get("Banishing", "—"))} / {E(d["skillpool"].get("Binding", "—"))}</dd>'
            f'<dt>Combat · Detection</dt><dd>{E(tr["spiritcombat"])} · {E(tr["spiritdetection"])}</dd>'
            f'<dt>Health · Illusion</dt><dd>{E(tr["spirithealth"])} · {E(tr["spiritillusion"])}</dd>'
            f'<dt>Manipulation</dt><dd>{E(tr["spiritmanipulation"])}</dd><dt>Spirit form</dt><dd>{E(tr["spiritform"])}</dd></dl></section>')
    mentor = ""
    if d["mentor"]:
        mentor = (f'<section class="card"><h2>Mentor spirit — {E(d["mentor"]["name"])}</h2>'
                  f'<p style="margin:0 0 3px"><b>Advantage.</b> {E(d["mentor"]["adv"])}</p>'
                  f'<p style="margin:0"><b>Disadvantage.</b> {E(d["mentor"]["dis"])}</p></section>')
    init = (f'<section class="card"><h2>Initiation</h2><dl class="kv"><dt>Grade</dt><dd>{E(d["initiategrade"])}</dd>'
            f'<dt>Metamagic</dt><dd>{E(", ".join(d["metamagics"]))}</dd><dt>Arts</dt><dd>{E(", ".join(d["arts"]))}</dd></dl></section>')
    sp_rows = ""
    for cat, ss in spell_groups(d):
        sp_rows += f'<tr><td colspan=8 style="border-bottom:.6pt solid #000;padding-top:4px"><b>{E(cat)}</b></td></tr>'
        for s in ss:
            dr = " ".join(
                (f"<u>{v}</u>" if f > d["mag"] else str(v)) for f in range(1, 7)
                if (v := drain_at(s["dv"], f)) is not None) or "—"
            sp_rows += (f"<tr><td><b>{E(s['name'])}</b><br><span class='small muted'>{E(s['descriptors'])} · {E(s['source'])}</span></td>"
                        f"<td class=n>{E(s['type'])}</td><td class=n>{E(s['range'])}</td><td class=n>{E(s['duration'])}</td>"
                        f"<td class=n>{E(s['damage']) if s['damage'] != 'None' else '—'}</td><td class=n>{E(s['dv'])}</td>"
                        f"<td class=n style='white-space:nowrap'>{dr}</td><td class=pool>{E(s['pool'])}</td></tr>")
    spells = (f'<section class="card"><h2>Spells</h2><table><tr><th>Spell</th><th class=n>Type</th><th class=n>Range</th>'
              f'<th class=n>Dur.</th><th class=n>Dmg</th><th class=n>DV</th><th class=n>Drain at F1–6</th><th class=n>Pool</th></tr>{sp_rows}</table>'
              f'<div class="small muted" style="margin-top:3px">Drain minimum 2. Underlined: Force above Magic ({d["mag"]}), drain is physical. '
              f'Resist with {E(tr["drainattributes"])} = {E(tr["drainvalue"])}.</div></section>')
    contacts = ('<section class="card"><h2>Contacts</h2><table><tr><th>Name</th><th>Role</th><th>Location</th><th class=n>C</th><th class=n>L</th></tr>'
                + "".join(f"<tr><td><b>{E(c['name'])}</b></td><td>{E(c['role'])}</td><td>{E(c['location'])}</td><td class=n>{E(c['c'])}</td><td class=n>{E(c['l'])}</td></tr>"
                          for c in d["contacts"]) + "</table></section>")
    life = ('<section class="card"><h2>Lifestyle</h2><table><tr><th>Lifestyle</th><th>Level</th><th class=n>Per month</th><th class=n>Paid</th></tr>'
            + "".join(f"<tr><td>{E(l['name'])}</td><td>{E(l['base'])}</td><td class=n>{E(l['monthly'])}¥</td><td class=n>{E(l['months'])} {E(l['increment'].lower())}</td></tr>"
                      for l in d["lifestyles"]) + "</table></section>")
    page3 = f'<div class="page">{spells}<div class="grid2"><div>{trad}{init}</div><div>{mentor}{life}</div></div>{contacts}</div>'

    # page 4: gear, drone, notes
    gitems = ""
    for g in d["gear"]:
        lis = "".join(
            f'<li class="d{min(dep, 2)}">{"<b>" if dep == 0 else ""}{E(gear_label(x))}{"</b>" if dep == 0 else ""}'
            f'{(" <span class=small>(" + commlink_stats(x) + ")</span>") if x["iscommlink"] and x["dr"] != "0" else ""}</li>'
            for dep, x in flat_gear([g]))
        gitems += f'<ul class="tree gear" style="margin-bottom:5px">{lis}</ul>'
    gear = f'<section class="card"><h2>Gear</h2><div class="cols2">{gitems}</div></section>'
    veh = ""
    for v in d["vehicles"]:
        st = "".join(f"<th class=n>{k}</th>" for k, _ in v["stats"])
        sv = "".join(f"<td class=n>{E(x)}</td>" for _, x in v["stats"])
        veh += (f'<section class="card"><h2>{"Drone" if v["drone"] else "Vehicle"} — {E(v["name"])}</h2>'
                f'<table><tr>{st}<th class=n>CM</th><th class=n>DR</th></tr><tr>{sv}<td class=n>{E(v["cm"])}</td><td class=n>{E(v["dr"])}</td></tr></table>'
                f'<div class="small" style="margin-top:3px">Gear: {E(", ".join(gear_label(g) for g in v["gear"]))}</div>'
                f'<div class="cm" style="grid-template-columns:repeat({num(v["cm"])},1fr);max-width:60%">{"<div></div>" * num(v["cm"])}</div></section>')
    money = (f'<section class="card"><h2>Karma &amp; nuyen</h2><dl class="kv"><dt>Karma available</dt><dd>{E(d["karma"])}</dd>'
             f'<dt>Career karma</dt><dd>{E(d["totalkarma"])}</dd><dt>Nuyen</dt><dd>{E(d["nuyen"])}¥</dd></dl>'
             f'<div class="lines small muted" style="margin-top:4px">Session log<div></div><div></div><div></div></div></section>')
    notes = (f'<section class="card note"><h2>Description</h2><p>{E(d["description"])}</p></section>'
             f'<section class="card note"><h2>Background</h2><p>{E(d["background"])}</p></section>'
             f'<section class="card"><h2>Notes</h2><div class="lines"><div></div><div></div><div></div><div></div><div></div><div></div></div></section>')
    page4 = f'<div class="page">{gear}<div class="grid2"><div>{veh}</div><div>{money}</div></div>{notes}</div>'

    return doc(f"{d['alias']} — clean sheet", css, page1 + page2 + page3 + page4)


# --------------------------------------------------------------------------
# Design 2: table-top dense
# --------------------------------------------------------------------------

def design2(d):
    font = "'Noto Sans Condensed', 'Liberation Sans Narrow', 'Arial Narrow', 'Noto Sans', 'Liberation Sans', sans-serif"
    css = page_css(f"{d['alias']} · {d['metatype']} · {'Magician' if d['magician'] else 'Runner'}", "", font,
                   size="7pt", margin="8mm 8mm 10mm 8mm") + f"""
body{{font:7.6pt/1.22 {font};font-stretch:condensed}}
.top{{display:flex;justify-content:space-between;align-items:baseline;white-space:nowrap;gap:8px;border-bottom:1.2pt solid #000;padding-bottom:2px;margin-bottom:4px}}
.top h1{{font-size:15pt;margin:0;display:inline;font-weight:800;letter-spacing:.02em}}
.top .who{{margin-left:6px}}
.top .nums span{{margin-left:10px}} .top .nums b{{font-size:9.5pt}}
.band{{display:grid;grid-template-columns:27% 43% 30%;gap:4px;margin-bottom:4px}}
.box{{border:.8pt solid #000;padding:2px 4px 3px;break-inside:avoid;margin-bottom:4px}}
.band .box{{margin:0}}
.box>h2{{font-size:7.2pt;margin:-2px -4px 2px;padding:1px 4px;border-bottom:.8pt solid #000;letter-spacing:.1em;font-weight:800}}
.cols{{columns:2;column-gap:5px}}
table{{border-collapse:collapse;width:100%}}
th{{font-size:6.4pt;text-align:left;font-weight:700;border-bottom:.5pt solid #000;padding:0 2px}}
td{{padding:.5px 2px;border-bottom:.3pt solid #bbb;vertical-align:top}}
.n{{text-align:center}} .pool{{font-weight:800;text-align:center;font-size:8.4pt}}
.at{{display:grid;grid-template-columns:repeat(4,1fr);gap:1px 3px}}
.at div{{border-bottom:.3pt solid #bbb;white-space:nowrap}} .at b{{font-size:9pt}} .at i{{font-style:normal;color:#555;font-size:6.4pt}}
.track{{display:flex;gap:1.5px;margin:1px 0 2px;align-items:center}}
.track span{{width:17px;height:13px;border:.8pt solid #000;font-size:5.6pt;text-align:center;line-height:12px}}
.track span.ov{{border-style:dashed}}
.track label{{width:38px;font-weight:700}}
.circ{{display:inline-block;width:10px;height:10px;border:.8pt solid #000;border-radius:50%;vertical-align:middle}}
.wl{{display:inline-block;border-bottom:.5pt solid #000;width:34px;height:9px}}
.slot{{display:inline-block;border:.6pt solid #000;width:54px;height:10px;margin-right:2px;vertical-align:middle}}
.ammo span{{display:inline-block;width:6px;height:6px;border:.5pt solid #000;margin-right:1px}}
.rng{{font-size:6.6pt;color:#333}}
.dr{{display:grid;grid-template-columns:repeat(6,13px);gap:0;}}
.dr span{{border:.4pt solid #888;text-align:center;font-size:6.6pt}}
.muted{{color:#555}} .tiny{{font-size:6.2pt}}
.ql{{display:grid;grid-template-columns:1fr 1fr;gap:0 6px}}
.tree{{margin:0;padding:0;list-style:none}} .tree li{{padding:0}}
.tree .d1{{padding-left:8px}} .tree .d2{{padding-left:16px}}
.lines div{{border-bottom:.5pt solid #999;height:11px}}
"""
    a = d["attrs"]
    at = "".join(f"<div>{k} <b>{E(a[k]['total'])}</b><i>/{E(a[k]['max'])}</i></div>" for k in ATTR_ORDER)
    at += f"<div>INI <b>{E(d['init'].split(' ')[0])}</b><i>+{E(d['init'].split('+')[-1].strip())}</i></div>"
    lims = (f"<div class='tiny' style='margin-top:2px'><b>LIMITS</b> Phy {E(d['limitphysical'])} · Men {E(d['limitmental'])} "
            f"· Soc {E(d['limitsocial'])} · Ast {E(d['limitastral'])}</div>"
            + "".join(f"<div class='tiny muted'>{E(lim[:3])} +{E(bn)} {E(n)} ({E(c)})</div>" for lim, n, bn, c in d["limitmods"]))
    attrs = f'<div class="box"><h2>ATTRIBUTES <span class="tiny muted">/natural max</span></h2><div class="at">{at}</div>{lims}</div>'

    def track(label, n, last, off, overflow=0):
        m = cm_marks(n, num(d["cmthreshold"]), off, last)
        s = "".join(f"<span>{E(m.get(i, ''))}</span>" for i in range(1, n + 1))
        s += "".join(f'<span class="ov">{"✝" if i == overflow else ""}</span>' for i in range(1, overflow + 1))
        return f'<div class="track"><label>{label}</label>{s}</div>'
    cm = f"""<div class="box"><h2>CONDITION MONITOR</h2>
<div class="tiny">Physical {E(d['physicalcm'])} + {E(d['cmoverflow'])} overflow</div>
{track('PHYS', num(d['physicalcm']), 'Down', num(d['physicalcmthresholdoffset']), num(d['cmoverflow']))}
<div class="tiny">Stun {E(d['stuncm'])} boxes</div>
{track('STUN', num(d['stuncm']), 'KO', num(d['stuncmthresholdoffset']))}
<div style="margin-top:2px"><b>EDGE</b> {'<span class="circ"></span> ' * d['edge']} &nbsp; <b>Init</b> <span class="wl"></span> &nbsp; <b>Wound</b> <span class="wl"></span></div>
<div style="margin-top:2px"><b>Sustain</b> <span class="tiny">−2 each</span> <span class="slot"></span><span class="slot"></span><span class="slot"></span></div>
<div style="margin-top:2px"><b>Notes</b><div class="lines"><div></div><div></div></div></div>
<div class="tiny" style="margin-top:2px">Init {E(d['init'])} · Astral {E(d['astralinit'])} · AR {E(d['matrixarinit'])} · Cold {E(d['matrixcoldinit'])} · Hot {E(d['matrixhotinit'])}</div></div>"""
    sp = d["skillpool"]
    pools = [("Defense (REA+INT)", d["dodge"]), ("Soak (BOD+Armor)", d["armordicephysical"]),
             ("Drain (" + d["tradition"]["drainattributes"] + ")", d["tradition"]["drainvalue"]),
             ("Spellcasting", sp.get("Spellcasting", "—")), ("Counterspelling", sp.get("Counterspelling", "—")),
             ("Perception", sp.get("Perception", "—")), ("Assensing", sp.get("Assensing", "—")),
             ("Composure", d["composure"]), ("Judge intentions", d["judgeintentions"]),
             ("Memory", d["memory"]), ("Surprise", d["surprise"]),
             ("Lift/carry", f"{d['liftandcarry']} ({d['liftweight']}/{d['carryweight']} kg)")]
    qp = ('<div class="box"><h2>QUICK POOLS</h2><table>'
          + "".join(f"<tr><td>{E(k)}</td><td class=pool>{E(v)}</td></tr>" for k, v in pools)
          + f"</table><div class='tiny' style='margin-top:1px'>Armor {E(d['armor'])} · Move {E(d['movement'].split(',')[0])}</div></div>")
    band = f'<div class="band">{attrs}{cm}{qp}</div>'

    top = (f'<div class="top"><div><h1>{E(d["alias"].upper())}</h1><span class="who">{E(d["metatype"])} · {E(profile(d))} · '
           f'Player {E(d["playername"])}</span></div><div class="nums">'
           f'<span>KARMA/TOTAL <b>{E(d["karma"])}/{E(d["totalkarma"])}</b></span><span>NUYEN <b>{E(d["nuyen"])}¥</b></span>'
           f'<span>ESSENCE <b>{E(d["totaless"])}</b></span><span>SC/NOT/PA <b>{E(d["totalstreetcred"])}/{E(d["totalnotoriety"])}/{E(d["totalpublicawareness"])}</b></span></div></div>')

    def wr(w, ub=False):
        n = ammo_count(w["ammo"])
        boxes = ('<div class="ammo">' + "<span></span>" * n + "</div>") if n and w["type"] == "Ranged" else ""
        rng = f'<div class="rng">{E(range_text(w))}</div>' if w["ranges"] else ""
        return (f"<tr><td class=pool>{E(w['pool'])}</td><td>{'↳ ' if ub else ''}<b>{E(w['name'])}</b>{rng}{boxes}</td>"
                f"<td class=n>{E(w['acc'])}</td><td class=n><b>{E(w['dv'])}</b></td><td class=n>{E(w['ap'])}</td>"
                f"<td class=n>{E(w['mode'])}</td><td class=n>{E(w['rc'])}</td><td class=n>{E(w['ammo']) if n else '—'}</td></tr>"
                + "".join(wr(u, True) for u in w["underbarrel"]))
    weapons = ('<div class="box"><h2>WEAPONS</h2><table><tr><th class=n>POOL</th><th>WEAPON</th><th class=n>ACC</th><th class=n>DV</th>'
               '<th class=n>AP</th><th class=n>MODE</th><th class=n>RC</th><th class=n>AMMO</th></tr>'
               + "".join(wr(w) for w in d["weapons"]) + "</table></div>")
    armor = ('<div class="box"><h2>ARMOR</h2>' + " · ".join(f"{'[x]' if x['equipped'] else '[ ]'} {E(x['name'])} {E(x['armor'])}" for x in d["armors"])
             + f" = <b>worn {E(d['armor'])}</b></div>")
    sk = "".join(f"<tr><td class=pool>{E(s['pool'])}</td><td>{E(s['name'])}{(' <span class=muted>(' + E(s['group']) + ')</span>') if s['group'] else ''}"
                 f"{(' [' + E(', '.join(s['specs'])) + ']') if s['specs'] else ''}</td><td class=n>{E(s['attr'])}</td><td class=n>{s['rating']}</td></tr>"
                 for s in d["skills"])
    groups = " · ".join(f"{E(g)} {E(r)}" for g, r in sorted(d["groups"].items()))
    skills = (f'<div class="box"><h2>ACTIVE SKILLS</h2><table><tr><th class=n>POOL</th><th>SKILL (GROUP)</th><th class=n>ATTR</th><th class=n>RTG</th></tr>{sk}</table>'
              f'<div class="tiny">Groups: {groups}</div></div>')
    kn = "".join(f"<tr><td class=pool>{'N' if s['native'] else E(s['pool'])}</td><td>{E(s['name'])} <span class=muted>{'lang.' if s['lang'] else E(s['category'])}</span></td>"
                 f"<td class=n>{E(s['attr'])}</td><td class=n>{'N' if s['native'] else s['rating']}</td></tr>" for s in d["knowledge"])
    know = f'<div class="box"><h2>KNOWLEDGE &amp; LANGUAGES</h2><table><tr><th class=n>POOL</th><th>SKILL</th><th class=n>ATTR</th><th class=n>RTG</th></tr>{kn}</table></div>'

    spr = ""
    for cat, ss in spell_groups(d):
        spr += f"<tr><td colspan=4 style='border-bottom:.6pt solid #000'><b>{E(cat.upper())}</b></td></tr>"
        for s in ss:
            cells = ""
            for f in range(1, 7):
                v = drain_at(s["dv"], f)
                cells += f"<span>{'—' if v is None else (f'<u><b>{v}</b></u>' if f > d['mag'] else v)}</span>"
            spr += (f"<tr><td><b>{E(s['name'])}</b><div class=tiny>{E(s['type'])} · {E(s['range'])} · {E(s['duration'])}"
                    f"{(' · ' + E(s['damage'])) if s['damage'] != 'None' else ''} · {E(s['descriptors'])}</div></td>"
                    f"<td class=n>{E(s['dv'])}</td><td><div class=dr>{cells}</div></td><td class=pool>{E(s['pool'])}</td></tr>")
    spells = (f'<div class="box"><h2>SPELLS</h2><table><tr><th>SPELL</th><th class=n>DV</th><th>DRAIN AT FORCE 1–6</th><th class=n>POOL</th></tr>{spr}</table>'
              f'<div class="tiny">Min 2. <u>Underlined</u>: Force &gt; Magic {d["mag"]} → physical drain. Resist {E(d["tradition"]["drainattributes"])} = <b>{E(d["tradition"]["drainvalue"])}</b>.</div></div>')
    tr = d["tradition"]
    trad = (f'<div class="box"><h2>TRADITION</h2><b>{E(tr["name"])}</b> · Drain {E(tr["drainattributes"])} = <b>{E(tr["drainvalue"])}</b><br>'
            f'Combat {E(tr["spiritcombat"])} · Detection {E(tr["spiritdetection"])} · Health {E(tr["spirithealth"])} · '
            f'Illusion {E(tr["spiritillusion"])} · Manipulation {E(tr["spiritmanipulation"])} · {E(tr["spiritform"])}<br>'
            + (f'<b>Mentor {E(d["mentor"]["name"])}</b>: {E(d["mentor"]["adv"])} <i>Disadv.:</i> {E(d["mentor"]["dis"])}<br>' if d["mentor"] else "")
            + f'<b>Initiate {E(d["initiategrade"])}</b>: {E(", ".join(d["metamagics"] + d["arts"]))}</div>')

    def ql(kind):
        return "".join(f"<div>{E(q['name'])}{(' (' + E(q['extra']) + ')') if q['extra'] else ''} <span class=muted>{E(q['bp'])}</span></div>"
                       for q in d["qualities"] if q["type"] == kind)
    quals = f'<div class="box"><h2>QUALITIES</h2><div class="ql"><div><b>+</b>{ql("Positive")}</div><div><b>−</b>{ql("Negative")}</div></div></div>'
    ware = ('<div class="box"><h2>AUGMENTATIONS</h2><table>'
            + "".join(f"<tr><td>{E(w['name'])}{(' R' + str(w['rating'])) if w['rating'] else ''} <span class=muted>{E(w['grade'])}</span></td><td class=n>{E(w['ess'])}</td></tr>" for w in d["ware"])
            + f"<tr><td><b>ESSENCE</b></td><td class=n><b>{E(d['totaless'])}</b></td></tr></table></div>")
    gl = "".join(f'<li class="d{min(dep, 2)}">{"└ " if dep else ""}{E(gear_label(x))}'
                 f'{(" [" + commlink_stats(x) + "]") if x["iscommlink"] and x["dr"] != "0" else ""}</li>'
                 for dep, x in flat_gear(d["gear"]))
    gear = f'<div class="box"><h2>GEAR</h2><ul class="tree">{gl}</ul></div>'
    veh = "".join(f'<div class="box"><h2>{"DRONE" if v["drone"] else "VEHICLE"}</h2><b>{E(v["name"])}</b><br>'
                  + " · ".join(f"{k[:3]} {E(x)}" for k, x in v["stats"])
                  + f' · CM {E(v["cm"])} · DR {E(v["dr"])}<br>{E(", ".join(gear_label(g) for g in v["gear"]))}'
                  + f'<div class="track">{"<span></span>" * num(v["cm"])}</div></div>' for v in d["vehicles"])
    cont = ('<div class="box"><h2>CONTACTS</h2><table><tr><th>NAME</th><th>ROLE / LOCATION</th><th class=n>C/L</th></tr>'
            + "".join(f"<tr><td><b>{E(c['name'])}</b></td><td>{E(c['role'])}{(' · ' + E(c['location'])) if c['location'] else ''}</td><td class=n>{E(c['c'])}/{E(c['l'])}</td></tr>" for c in d["contacts"])
            + "</table></div>")
    life = "".join(f'<div class="box"><h2>LIFESTYLE</h2>{E(l["name"])} ({E(l["base"])}) · <b>{E(l["monthly"])}¥/mo</b> · paid {E(l["months"])} {E(l["increment"].lower())}</div>' for l in d["lifestyles"])
    notes = (f'<div class="box"><h2>NOTES</h2><div class=tiny>{E(d["height"])}, {E(d["weight"])}, eyes {E(d["eyes"])}, hair {E(d["hair"])}. '
             f'{E(d["description"])}</div><div class="lines"><div></div><div></div><div></div><div></div><div></div></div></div>')
    body = (top + band + '<div class="cols">' + weapons + armor + skills + know + spells + trad
            + quals + ware + gear + veh + cont + life + notes + "</div>")
    return doc(f"{d['alias']} — dense sheet", css, body)


# --------------------------------------------------------------------------
# Design 3: Shadowrun flavour "runner dossier"
# --------------------------------------------------------------------------

def design3(d):
    mono = "'Noto Sans Mono', 'DejaVu Sans Mono', 'Liberation Mono', monospace"
    sans = "'Noto Sans', 'Liberation Sans', sans-serif"
    fid = f"{d['alias'].upper()}-{DOSSIER_FILE_ID}"
    css = page_css(f"// RUNNER DOSSIER {fid}", "// ", mono, size="7pt") + f"""
@page {{ @bottom-right {{ content: "// PAGE " counter(page) " / " counter(pages); }} }}
body{{font:8pt/1.3 {sans}}}
.m{{font-family:{mono}}}
.rule{{display:flex;justify-content:space-between;font:7.4pt {mono};border-top:1.6pt solid #000;border-bottom:.6pt solid #000;padding:2px 0;letter-spacing:.06em;margin-bottom:6px}}
.hero{{display:grid;grid-template-columns:34mm 1fr 52mm;gap:10px;margin-bottom:6px}}
.pic{{position:relative;padding:5px}}
.pic img{{width:100%;display:block;filter:grayscale(1) contrast(1.05)}}
.pic i{{position:absolute;width:10px;height:10px;border:0 solid #000}}
.pic i.a{{top:0;left:0;border-top-width:1.4pt;border-left-width:1.4pt}} .pic i.b{{top:0;right:0;border-top-width:1.4pt;border-right-width:1.4pt}}
.pic i.c{{bottom:12px;left:0;border-bottom-width:1.4pt;border-left-width:1.4pt}} .pic i.d{{bottom:12px;right:0;border-bottom-width:1.4pt;border-right-width:1.4pt}}
.pic .cap{{font:6.4pt {mono};text-align:center;margin-top:3px;letter-spacing:.08em}}
.id{{position:relative}}
.id h1{{font:700 26pt/1 {mono};margin:0 0 6px}}
.stamp{{position:absolute;right:4px;top:2px;transform:rotate(-9deg);border:1.6pt solid #000;padding:1px 7px;
  font:700 11pt {mono};letter-spacing:.2em;color:transparent;-webkit-text-stroke:.7pt #000;border-radius:3px}}
.lv{{display:grid;grid-template-columns:24mm 1fr;font:7.6pt/1.45 {mono}}}
.lv .k{{color:#444}} .lv .k::after{{content:" ..";color:#999}}
.side{{border:1pt solid #000;padding:4px 6px;font:7.6pt/1.6 {mono}}}
.side div{{display:flex;justify-content:space-between;border-bottom:.4pt dotted #888}}
.side b{{font-size:9pt}}
h2{{font:700 8.6pt {mono};letter-spacing:.08em;margin:8px 0 3px;display:flex;align-items:center;gap:6px}}
h2::after{{content:"";flex:1;border-top:.8pt dashed #000}}
h2 .no{{border:.8pt solid #000;padding:0 3px}}
section{{break-inside:avoid}}
.page{{break-before:page}}
.two{{display:grid;grid-template-columns:1fr 1fr;gap:0 12px}}
.ag{{display:grid;grid-template-columns:repeat(2,1fr);gap:1px 14px;font:7.8pt {mono}}}
.ag div{{display:grid;grid-template-columns:9mm 7mm 1fr;align-items:center;border-bottom:.4pt dotted #999;padding:1px 0}}
.pips span{{display:inline-block;width:7px;height:7px;border:.7pt solid #000;margin-right:1.5px;vertical-align:middle}}
.pips span.f{{background:repeating-linear-gradient(45deg,#000 0 .8px,#fff .8px 2.2px)}}
.legend{{font:6.6pt {mono};color:#444;margin-top:2px}}
table{{border-collapse:collapse;width:100%;font:7.4pt {mono}}}
th{{text-align:left;font-weight:700;border-bottom:.8pt solid #000;padding:1px 3px;font-size:6.8pt}}
td{{padding:1px 3px;border-bottom:.4pt dotted #999;vertical-align:top}}
.n{{text-align:center}} .p{{font-weight:700;white-space:nowrap}}
.cm{{display:flex;flex-wrap:wrap;gap:2px;margin:2px 0 5px}}
.cm span{{width:22px;height:15px;border:.9pt solid #000;font:6pt/14px {mono};text-align:center}}
.cm span.ov{{border-style:dashed}}
.edge span{{display:inline-block;width:13px;height:13px;border:.9pt solid #000;border-radius:50%;vertical-align:middle;margin-right:3px}}
.kv{{font:7.4pt/1.5 {mono}}} .kv div{{display:flex;justify-content:space-between;border-bottom:.4pt dotted #999}}
.tree{{font:7.4pt/1.45 {mono};margin:0;padding:0;list-style:none;white-space:pre}}
.small{{font-size:6.8pt}} .muted{{color:#444}}
.lines div{{border-bottom:.5pt solid #999;height:14px}}
.prose{{font-size:7.8pt}}
"""
    a = d["attrs"]
    pic = (f'<div class="pic"><i class="a"></i><i class="b"></i><i class="c"></i><i class="d"></i>'
           f'<img src="{d["portrait_grey"]}" alt=""><div class="cap">IMG 01 // {E(d["alias"].upper())}</div></div>'
           if d["portrait_grey"] else '<div></div>')
    h = d["handler"]
    lv = [("LEGAL NAME", d["name"]), ("METATYPE", d["metatype"]), ("PROFILE", profile(d)),
          ("MENTOR", d["mentor"]["name"] if d["mentor"] else "—"), ("SIN STATUS", d["sin"]),
          ("PHYSICAL", f"{d['gender']}, {d['age']} · {d['height']}, {d['weight']} · eyes {d['eyes']} · hair {d['hair']} · skin {d['skin']}"),
          ("HANDLER", f"{h['name']} ({h['role']}, {h['location']}) C{h['c']}/L{h['l']}" if h else "—"),
          ("PLAYER", d["playername"])]
    ident = (f'<div class="id"><h1>&gt; {E(d["alias"].upper())}</h1>'
             + ('<div class="stamp">AWAKENED</div>' if d["magician"] or d["adept"] else "")
             + '<div class="lv">' + "".join(f'<span class="k">{k}</span><span>{E(v)}</span>' for k, v in lv) + "</div></div>")
    side = ('<div class="side">' + "".join(f"<div><span>{k}</span><b>{E(v)}</b></div>" for k, v in [
        ("KARMA", d["karma"]), ("CAREER KARMA", d["totalkarma"]), ("NUYEN", d["nuyen"] + "¥"),
        ("ESSENCE", d["totaless"]), ("STREET CRED", d["totalstreetcred"]),
        ("NOTORIETY", d["totalnotoriety"]), ("PUBLIC AWARENESS", d["totalpublicawareness"])]) + "</div>")
    rule = (f'<div class="rule"><span>SHADOWRUN 5E // RUNNER DOSSIER</span><span>FILE {E(fid)}</span>'
            f'<span>{"CAREER" if d["career"] else "CREATION"} // {E(d["buildmethod"].upper())}</span></div>')
    hero = rule + f'<div class="hero">{pic}{ident}{side}</div>'

    def sec(no, title, inner):
        return f'<section><h2><span class="no">{no}</span>{title}</h2>{inner}</section>'

    ag = ""
    for k in ATTR_ORDER:
        x = a[k]
        tot, mx = x["total"], num(x["max"])
        if k == "ESS":
            pips = f'<span class="small muted">max {E(x["max"])}</span>'
        else:
            tv = num(tot)
            pips = '<span class="pips">' + "".join(f'<span class="{"f" if i <= tv else ""}"></span>' for i in range(1, mx + 1)) + "</span>"
        ag += f"<div><b>{k}</b><b>{E(tot)}</b>{pips}</div>"
    lims = (f'<div class="kv" style="margin-top:4px"><div><span>LIMITS  PHY/MEN/SOC/AST</span><b>{E(d["limitphysical"])} / {E(d["limitmental"])} / {E(d["limitsocial"])} / {E(d["limitastral"])}</b></div>'
            + "".join(f'<div class="small"><span>  +{E(bn)} {E(lim[:3]).upper()} {E(n)}</span><span>{E(c)}</span></div>' for lim, n, bn, c in d["limitmods"]) + "</div>")
    s01 = sec("01", "ATTRIBUTES", f'<div class="ag">{ag}</div><div class="legend">■ natural · □ to max</div>{lims}')
    der = [("INITIATIVE", d["init"]), ("ASTRAL INIT", d["astralinit"]),
           ("MATRIX AR/COLD/HOT", f"{d['matrixarinit']} | {d['matrixcoldinit']} | {d['matrixhotinit']}"),
           ("DEFENSE (REA+INT)", f"[{d['dodge']}]"), ("SOAK (BOD+ARMOR)", f"[{d['armordicephysical']}]"),
           ("ARMOR WORN", d["armor"]), ("DRAIN (" + d["tradition"]["drainattributes"] + ")", f"[{d['tradition']['drainvalue']}]"),
           ("SURPRISE", f"[{d['surprise']}]"), ("COMPOSURE", f"[{d['composure']}]"),
           ("JUDGE INTENTIONS", f"[{d['judgeintentions']}]"), ("MEMORY", f"[{d['memory']}]"),
           ("LIFT/CARRY", f"[{d['liftandcarry']}] {d['liftweight']}/{d['carryweight']} kg"),
           ("MOVEMENT", d["movement"].split(",")[0]),
           ("RECOVERY PHYS/STUN", f"[{d['physicalcmnaturalrecovery']}] day | [{d['stuncmnaturalrecovery']}] hr")]
    s02 = sec("02", "DERIVED", '<div class="kv">' + "".join(f"<div><span>{k}</span><b>{E(v)}</b></div>" for k, v in der) + "</div>")

    def cmrow(n, last, off, overflow=0):
        m = cm_marks(n, num(d["cmthreshold"]), off, last)
        s = "".join(f"<span>{E(m.get(i, ''))}</span>" for i in range(1, n + 1))
        s += "".join(f'<span class="ov">{"✝" if i == overflow else "+"}</span>' for i in range(1, overflow + 1))
        return f'<div class="cm">{s}</div>'
    s03 = sec("03", "CONDITION", f"""<div class="m small">PHYSICAL [{E(d['physicalcm'])}] + OVERFLOW [{E(d['cmoverflow'])}]</div>
{cmrow(num(d['physicalcm']), 'Down', num(d['physicalcmthresholdoffset']), num(d['cmoverflow']))}
<div class="m small">STUN [{E(d['stuncm'])}]</div>{cmrow(num(d['stuncm']), 'KO', num(d['stuncmthresholdoffset']))}
<div class="edge m small">EDGE {'<span></span>' * d['edge']} {E(d['edgeremaining'])}/{d['edge']} LEFT &nbsp; SUSTAINING ____ ____ ____</div>""")
    rs = ""
    for title, items in RESIST_ROWS:
        vals = " ".join((f"{lbl} [{d[k]}]" if lbl else f"[{d[k]}]") for lbl, k in items)
        rs += f"<tr><td>{E(title.upper())}</td><td>{E(vals)}</td></tr>"
    s03b = sec("03b", "RESIST", f"<table>{rs}</table>")
    s13 = sec("03c", "FIELD NOTES", f'<div class="prose">{E(d["description"])}</div><div class="lines"><div></div><div></div><div></div><div></div></div>')
    page1 = hero + f'<div class="two"><div>{s01}{s03}</div><div>{s02}</div></div>{s03b}{s13}'

    def skrows(ss):
        return "".join(f"<tr><td class=p>[{E(s['pool'])}]</td><td>{E(s['name'])}{(' (' + E(', '.join(s['specs'])) + ')') if s['specs'] else ''}</td>"
                       f"<td class=n>{E(s['attr'])}</td><td class=n>{s['rating']}</td><td class=muted>{E(s['group'])}</td></tr>" for s in ss)
    half = (len(d["skills"]) + 1) // 2
    hd = "<tr><th>POOL</th><th>SKILL</th><th class=n>ATTR</th><th class=n>RTG</th><th>GROUP</th></tr>"
    groups = " · ".join(f"{E(g.upper())} {E(r)}" for g, r in sorted(d["groups"].items()))
    s04 = sec("04", "SKILLS", f'<div class="two"><table>{hd}{skrows(d["skills"][:half])}</table><table>{hd}{skrows(d["skills"][half:])}</table></div>'
              f'<div class="m small muted">GROUPS: {groups}</div>')
    kn = "".join(f"<tr><td class=p>[{'N' if s['native'] else E(s['pool'])}]</td><td>{E(s['name'])}</td><td class=muted>{'LANGUAGE' if s['lang'] else E(s['category'].upper())}</td>"
                 f"<td class=n>{E(s['attr'])}</td><td class=n>{'N' if s['native'] else s['rating']}</td></tr>" for s in d["knowledge"])
    s05 = sec("05", "KNOWLEDGE // LANGUAGES", f"<table><tr><th>POOL</th><th>SKILL</th><th>TYPE</th><th class=n>ATTR</th><th class=n>RTG</th></tr>{kn}</table>")

    def qr(kind, sign):
        return "".join(f"<tr><td>{sign} {E(q['name'])}{(' :: ' + E(q['extra'])) if q['extra'] else ''}</td><td class=n>{E(q['bp'])}</td></tr>"
                       for q in d["qualities"] if q["type"] == kind)
    s06 = sec("06", "QUALITIES", f'<table><tr><th>QUALITY</th><th class=n>KARMA</th></tr>{qr("Positive", "+")}{qr("Negative", "−")}</table>')

    def wrow(w, ub=False):
        rng = f"<div class='small muted'>  └─ {E(range_text(w, ' | '))}</div>"
        return (f"<tr><td class=p>[{E(w['pool'])}]</td><td>{'└─ ' if ub else ''}<b>{E(w['name'])}</b>{rng}</td><td class=n>{E(w['acc'])}</td>"
                f"<td class=n><b>{E(w['dv'])}</b></td><td class=n>{E(w['ap'])}</td><td class=n>{E(w['mode'])}</td><td class=n>{E(w['rc'])}</td>"
                f"<td class=n>{E(w['ammo']) if ammo_count(w['ammo']) else '—'}</td></tr>" + "".join(wrow(u, True) for u in w["underbarrel"]))
    s07 = sec("07", "COMBAT // WEAPONS", "<table><tr><th>POOL</th><th>WEAPON</th><th class=n>ACC</th><th class=n>DV</th><th class=n>AP</th>"
              "<th class=n>MODE</th><th class=n>RC</th><th class=n>AMMO</th></tr>" + "".join(wrow(w) for w in d["weapons"]) + "</table>")
    arm = "".join(f"<div><span>{'[x]' if x['equipped'] else '[ ]'} {E(x['name'])}</span><b>{E(x['armor'])}</b></div>" for x in d["armors"])
    s07b = sec("07b", "COMBAT // ARMOR", f'<div class="kv">{arm}<div><span>WORN TOTAL</span><b>{E(d["armor"])}</b></div></div>')
    page2 = f'<div class="page">{s04}<div class="two"><div>{s05}</div><div>{s06}</div></div>{s07}{s07b}</div>'

    ware = "".join(f"<div><span>{E(w['name'])}{(' R' + str(w['rating'])) if w['rating'] else ''} :: {E(w['grade'].upper())}</span><b>{E(w['ess'])}</b></div>" for w in d["ware"])
    s08 = sec("09", "AUGMENTATIONS", f'<div class="kv">{ware}<div><span>ESSENCE REMAINING</span><b>{E(d["totaless"])}</b></div></div>')
    tr = d["tradition"]
    spr = ""
    for cat, ss in spell_groups(d):
        spr += f"<tr><td colspan=10><b>:: {E(cat.upper())}</b></td></tr>"
        for s in ss:
            cells = ""
            for f in range(1, 7):
                v = drain_at(s["dv"], f)
                cells += f"<td class=n>{'—' if v is None else (f'<u><b>{v}</b></u>' if f > d['mag'] else v)}</td>"
            spr += (f"<tr><td class=p>[{E(s['pool'])}]</td><td>{E(s['name'])}<div class='small muted'>{E(s['type'])} | {E(s['range'])} | {E(s['duration'])}"
                    f"{(' | ' + E(s['damage'])) if s['damage'] != 'None' else ''}</div></td><td class=n>{E(s['dv'])}</td>{cells}</tr>")
    mm = d["mentor"]
    s09 = sec("08", "MAGIC // SPELLS", f"""<table><tr><th>POOL</th><th>SPELL</th><th class=n>DV</th><th class=n>F1</th><th class=n>F2</th><th class=n>F3</th><th class=n>F4</th><th class=n>F5</th><th class=n>F6</th></tr>{spr}</table>
<div class="m small muted">DRAIN MIN 2 · <u>UNDERLINED</u> = FORCE &gt; MAGIC {d['mag']} → PHYSICAL · RESIST {E(tr['drainattributes'])} [{E(tr['drainvalue'])}]</div>""")
    s09b = sec("08b", "MAGIC // TRADITION", f"""<div class="kv"><div><span>TRADITION</span><b>{E(tr['name'].upper())}</b></div>
<div><span>COMBAT / DETECTION</span><span>{E(tr['spiritcombat'])} / {E(tr['spiritdetection'])}</span></div>
<div><span>HEALTH / ILLUSION</span><span>{E(tr['spirithealth'])} / {E(tr['spiritillusion'])}</span></div>
<div><span>MANIPULATION</span><span>{E(tr['spiritmanipulation'])}</span></div>
<div><span>INITIATE GRADE</span><b>{E(d['initiategrade'])}</b></div>
<div><span>METAMAGIC / ARTS</span><span>{E(', '.join(d['metamagics'] + d['arts']))}</span></div></div>"""
                + (f'<div class="small" style="margin-top:3px"><b>MENTOR {E(mm["name"].upper())}</b> + {E(mm["adv"])} − {E(mm["dis"])}</div>' if mm else ""))

    lines = []
    for dep, x in flat_gear(d["gear"]):
        pre = "" if dep == 0 else "   " * (dep - 1) + "└─ "
        extra = f"  [{commlink_stats(x)}]" if x["iscommlink"] and x["dr"] != "0" else ""
        lines.append(E(pre + gear_label(x) + extra))
    s10 = sec("10", "GEAR", '<div class="tree">' + "\n".join(lines) + "</div>")
    vh = ""
    for v in d["vehicles"]:
        vh += (f'<div class="kv"><div><span><b>{E(v["name"])}</b></span><span>{E(v["category"])}</span></div>'
               f'<div><span>{" ".join(f"{k[:3].upper()} {E(x)}" for k, x in v["stats"])}</span><span>CM [{E(v["cm"])}] DR {E(v["dr"])}</span></div></div>'
               f'<div class="tree">' + "\n".join("└─ " + E(gear_label(g)) for g in v["gear"]) + "</div>"
               f'<div class="cm">{"<span></span>" * num(v["cm"])}</div>')
    s10b = sec("10b", "DRONES", vh)
    ct = "".join(f"<tr><td>{E(c['name'])}</td><td>{E(c['role'] or '—')}</td><td>{E(c['location'] or '—')}</td><td class=n>{E(c['c'])}/{E(c['l'])}</td></tr>" for c in d["contacts"])
    s11 = sec("11", "CONTACTS", f"<table><tr><th>NAME</th><th>ROLE</th><th>LOCATION</th><th class=n>C/L</th></tr>{ct}</table>")
    lf = "".join(f"<div><span>{E(l['name'])} :: {E(l['base'].upper())}</span><b>{E(l['monthly'])}¥/MO</b></div>" for l in d["lifestyles"])
    s12 = sec("09b", "LIFESTYLE // ASSETS", f'<div class="kv">{lf}<div><span>NUYEN ON HAND</span><b>{E(d["nuyen"])}¥</b></div>'
              f'<div><span>KARMA / CAREER</span><b>{E(d["karma"])} / {E(d["totalkarma"])}</b></div></div>')
    page3 = (f'<div class="page">{s09}<div class="two"><div>{s09b}{s08}{s12}</div><div>{s10}{s10b}</div></div>{s11}</div>')
    return doc(f"{d['alias']} — runner dossier", css, page1 + page2 + page3)


# --------------------------------------------------------------------------
# Current sheet + rendering
# --------------------------------------------------------------------------

def shrink_current(src, dst, d):
    s = open(src, encoding="utf-8").read()
    if d["portrait"]:
        s = re.sub(r"data:image/[a-z]+;base64,[A-Za-z0-9+/=\s]+", d["portrait"], s)
        # The sheet shows the portrait at its natural size (width: auto), so
        # pin the original pixel width to keep the layout of the real output.
        if d.get("portrait_px"):
            s = s.replace('class="mugshot"', f'class="mugshot" style="width:{d["portrait_px"]}px"')
    open(dst, "w", encoding="utf-8").write(s)


def find_chrome():
    hits = sorted(glob.glob(os.path.expanduser(
        "~/.cache/ms-playwright/chromium_headless_shell-*/*/chrome-headless-shell")))
    if not hits:
        sys.exit("chrome-headless-shell not found under ~/.cache/ms-playwright")
    return hits[-1]


def render(out, names):
    chrome = find_chrome()
    os.makedirs(os.path.join(out, "png"), exist_ok=True)
    for n in names:
        h = os.path.join(out, n + ".html")
        pdf = os.path.join(out, n + ".pdf")
        subprocess.run(["nice", "-n", "19", chrome, "--headless", "--no-sandbox", "--disable-gpu",
                        "--no-pdf-header-footer", f"--print-to-pdf={pdf}", "file://" + h],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for old in glob.glob(os.path.join(out, "png", n + "-*.png")):
            os.remove(old)
        subprocess.run(["nice", "-n", "19", "pdftoppm", "-r", "90", "-png", pdf,
                        os.path.join(out, "png", n)], check=True)
        pages = subprocess.run(["pdfinfo", pdf], capture_output=True, text=True).stdout
        print(n, re.search(r"Pages:\s+(\d+)", pages).group(1), "pages")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("xml")
    ap.add_argument("--out", default=HERE)
    ap.add_argument("--current-in", help="default-sheet HTML from chummer-cli sheet")
    ap.add_argument("--render", action="store_true")
    o = ap.parse_args()
    d = load(o.xml)
    os.makedirs(o.out, exist_ok=True)
    names = []
    for n, fn in (("design-1-clean", design1), ("design-2-dense", design2), ("design-3-flavour", design3)):
        with open(os.path.join(o.out, n + ".html"), "w", encoding="utf-8") as f:
            f.write(fn(d))
        names.append(n)
    if o.current_in:
        shrink_current(o.current_in, os.path.join(o.out, "current.html"), d)
        names.insert(0, "current")
    elif os.path.exists(os.path.join(o.out, "current.html")):
        names.insert(0, "current")
    if o.render:
        render(o.out, names)


if __name__ == "__main__":
    main()
