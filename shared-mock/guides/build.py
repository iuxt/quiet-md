#!/usr/bin/env python3
"""Build the 开始指南 sample library from shared-mock/guides/src.

usage: python3 shared-mock/guides/build.py <out_library_dir>

Each src/*.xml is one document (file name order = reading order). Placeholders:
  {{doc:key}}                       @文档 link to the guide whose KEYS entry is key
  {{reminder:YYYY-MM-DD HH:MM}}     timed date reminder, notified when it is due
  {{reminder-day:YYYY-MM-DD}}       whole-day reminder, notified 09:00 that day
  {{countdown:YYYY-MM-DD HH:MM}}    countdown block to that moment (blue)
  {{button:Action|url|text|color}}  inline button (OpenLink / DuplicatePage)
Assets under guides/assets are copied to the library's resource directories.

The output is a Quiet.md library with virtual folder identities in quiet.json,
fixed Notes/<UUID>.xml bodies and relative document links.
"""
import datetime as dt, json, re, shutil, sys, time, uuid
from pathlib import Path

here = Path(__file__).resolve().parent
out = Path(sys.argv[1]).resolve()
NS = uuid.UUID("6f7c1e9a-9d0b-4a51-8c7e-2a1d3b4c5e6f")
# The top level, in order; every entry gets its place, so the sidebar and
# Home's manual order (the default) read the same.
TOP = ["welcome", "writing-guides", "examples", "organize", "data", "personalize", "practice"]
# file stem -> (key, parent key, pinned); a parent is another document key
KEYS = {
    "01-welcome": ("welcome", None, False),
    "01a-writing-guides": ("writing-guides", None, False),
    "02-writing": ("writing", "writing-guides", False),
    "03-blocks": ("blocks", "writing-guides", False),
    "04-tables": ("tables", "writing-guides", False),
    "05-code": ("code", "writing-guides", False),
    "06-media": ("media", "writing-guides", False),
    "07-organize": ("organize", None, False),
    "07-examples": ("examples", None, False),
    "07a-reading-list": ("reading-list", "examples", False),
    "07b-weekly": ("weekly", "examples", False),
    "08-data": ("data", None, False),
    "09-personalize": ("personalize", None, False),
    "10-practice": ("practice", None, False),
}
ids = {k: str(uuid.uuid5(NS, k)) for k, _, _ in KEYS.values()}
srcs = {KEYS[p.stem][0]: p.read_text() for p in sorted((here / "src").glob("*.xml"))}
missing = set(ids) - set(srcs)
assert not missing, f"no source for {missing}"

def esc(s): return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
def plain_title(x):
    t = re.search(r"<title[^>]*>(.*?)</title>", x, re.S).group(1)
    return re.sub(r"<[^>]+>", "", t).replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
titles = {k: plain_title(x) for k, x in srcs.items()}
local = lambda s: dt.datetime.strptime(s, "%Y-%m-%d %H:%M").astimezone()
ms = lambda d: int(d.timestamp() * 1000)
now = int(time.time())

def rel_path(frm, to):
    a, b = [p for p in frm.split("/") if p], [p for p in to.split("/") if p]
    n = 0
    while n < min(len(a), len(b)) and a[n] == b[n]: n += 1
    return "/".join([".."] * (len(a) - n) + b[n:])
# Body paths do not depend on the visible tree or titles.
paths = {k: f"Notes/{ids[k]}.xml" for k, _, _ in KEYS.values()}
here_dir = ""

def fill(x):
    def doc(m):
        k = m.group(1)
        assert k in ids, f"unknown doc {k}"
        return f'<cite type="doc" href="{esc(rel_path(here_dir, paths[k])).replace(chr(34), "&quot;")}" file-type="docx">{esc(titles[k])}</cite>'
    def rem(m):
        d = local(m.group(1))
        return f'<time expire-time="{ms(d)}" notify-time="{ms(d)}" should-notify="true">{d:%Y-%m-%d %H:%M}</time>'
    def rem_day(m):
        d = local(m.group(1) + " 23:59")
        n = local(m.group(1) + " 09:00")
        return f'<time expire-time="{ms(d)}" notify-time="{ms(n)}" should-notify="true" is-whole-day="true">{d:%Y-%m-%d}</time>'
    def countdown(m):
        d = local(m.group(1))
        start = now * 1000
        dur = (ms(d) - start) // 1000 + 1
        return f'<countdown duration="{dur}" start-time="{start}" notify="true" color="#4E83FD" timing="date" date="{d:%Y-%m-%d}" time="{d:%H:%M}"/>'
    def button(m):
        action, url, text, color = m.group(1).split("|")
        src = f' src="{esc(url)}"' if url else ""
        return f'<button action="{action}"{src} background-color="{color}">{esc(text)}</button>'
    x = re.sub(r"\{\{doc:([a-z-]+)\}\}", doc, x)
    x = re.sub(r"\{\{reminder:([0-9-]+ [0-9:]+)\}\}", rem, x)
    x = re.sub(r"\{\{reminder-day:([0-9-]+)\}\}", rem_day, x)
    x = re.sub(r"\{\{countdown:([0-9-]+ [0-9:]+)\}\}", countdown, x)
    x = re.sub(r"\{\{button:([^}]+)\}\}", button, x)
    assert "{{" not in x, re.search(r"\{\{[^}]*\}\}", x).group(0)
    # A cover is a file in the library: bring the app's gallery image along.
    def cover(m):
        name = m.group(1)
        covers.add(name)
        return f'cover="Images/guides/covers/{name}"'
    return re.sub(r'cover="covers/([^"]+)"', cover, x)

covers = set()

if out.exists() and any(out.iterdir()):
    sys.exit(f"{out} is not empty")
out.mkdir(parents=True, exist_ok=True)
(out / "Notes").mkdir()
index = {}
order = list(KEYS.values())
inside = {}
for i, (k, parent, pinned) in enumerate(order):
    here_dir = paths[k].rpartition("/")[0]
    (out / paths[k]).write_text(fill(srcs[k]))
    # Reading order is written out: TOP places the top level, and each parent's
    # children are numbered in source order.
    t = now - 60 * i
    meta = {"id": ids[k], "created": now - 3600 + i, "modified": t}
    if pinned: meta["pinned"] = True
    if parent:
        assert parent in ids, f"{k}: unknown parent {parent}"
        inside[parent] = inside.get(parent, 0) + 1
        meta["order"] = inside[parent]
    else:
        assert k in TOP, f"{k} is missing from TOP"
    index[paths[k]] = meta
assert set(TOP) == {k for k, p, _ in KEYS.values() if not p}, "TOP must list every top-level document"
for i, k in enumerate(TOP):
    index[paths[k]]["root_order"] = i + 1
stamp = lambda t: dt.datetime.fromtimestamp(t, dt.timezone.utc).isoformat().replace("+00:00", "Z")
notes = []
for k, parent, _ in order:
    meta = index[paths[k]]
    options = {key: value for key, value in meta.items() if key not in ("id", "created", "modified")}
    notes.append({"id": ids[k], "title": titles[k], "filename": f"{ids[k]}.xml", "parentId": ids[parent] if parent else None,
                  "createdAt": stamp(meta["created"]), "updatedAt": stamp(meta["modified"]), "options": options})
manifest = {"version": 3, "notes": sorted(notes, key=lambda n: n["id"]), "trash": []}
(out / "quiet.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
for asset in sorted((here / "assets").rglob("*")):
    if not asset.is_file() or asset.name.startswith("."):
        continue
    ext = asset.suffix.lower()
    if ext in {".jpeg", ".jpg", ".png", ".gif", ".webp", ".bmp"}:
        folder = "Images"
    elif ext == ".excalidraw":
        folder = "Drawings"
    else:
        folder = "Files"
    dest = out / folder / "guides" / asset.relative_to(here / "assets")
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(asset, dest)
cover_dir = out / "Images" / "guides" / "covers"
cover_dir.mkdir(parents=True, exist_ok=True)
for name in sorted(covers):
    shutil.copy2(here.parent.parent / "assets" / "covers" / name, cover_dir / name)
print(out)
