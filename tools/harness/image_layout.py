"""Image drag/drop regression in temporary libraries. Build with devtools; needs Pillow."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

from PIL import Image, ImageChops

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-image-layout-"))
note = "33333333-3333-4333-8333-333333333333"
colors = dict(a=(170, 55, 55), b=(50, 150, 75), c=(50, 90, 180), d=(180, 145, 40))
source = work / "source"
(source / ".quiet").mkdir(parents=True)
(source / "notes").mkdir()
(source / "assets").mkdir()
(source / ".quiet/format.json").write_text('{"format":6}')
(source / ".quiet/preferences.json").write_text(json.dumps({
    "theme": "craft", "appearance": "light", "english": True, "seen_version": "0.1.12",
    "auto_update": False, "quick_hotkey": False,
}))
(source / "quiet.json").write_text(json.dumps({"version": 2, "folders": [], "notes": [{
    "id": note, "title": "Image layout", "filename": f"{note}.xml", "folderId": None,
    "options": {"opened": 1},
}], "trash": []}))
for name, color in colors.items():
    Image.new("RGB", (240, 120), color).save(source / f"assets/{name}.png")
body = '<title>Image layout</title>\n<p>Arrange images</p>\n'
body += ''.join(f'<img src="assets/{name}.png" width="240" height="120" align="left"/>\n' for name in colors)
body += '<p>End</p>\n'
(source / f"notes/{note}.xml").write_text(body)


def run(name, lib, steps="", width=1200):
    folder = work / name
    script = work / f"{name}.txt"
    script.write_text(f"resize {width} 950\nwait 1400\nmove {width-40} 900\nwait 200\nshot before\n"
                      + steps + f"\nmove {width-40} 900\nwait 2300\nshot after\n")
    subprocess.run([sys.executable, str(here / "run.py"), str(script), str(lib), str(folder)],
                   check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
    return folder


def boxes(folder, frame="after", width=1200):
    im = Image.open(folder / f"{frame}.png").convert("RGB")
    scale = im.width / width
    out = {}
    for name, color in colors.items():
        channels = ImageChops.difference(im, Image.new("RGB", im.size, color)).split()
        masks = [c.point(lambda v: 255 if v == 0 else 0) for c in channels]
        box = ImageChops.multiply(ImageChops.multiply(masks[0], masks[1]), masks[2]).getbbox()
        assert box, f"Missing image {name}: {folder}/{frame}.png"
        out[name] = tuple(v / scale for v in box)
    return out


def layout(folder):
    xml = (folder / "lib" / f"notes/{note}.xml").read_text()
    doc = ET.fromstring("<doc>" + xml + "</doc>")

    def node(b):
        if b.tag == "img":
            assert b.attrib["width"] == "240" and b.attrib["height"] == "120"
            return Path(b.attrib["src"]).stem
        if b.tag == "grid":
            ratios = [float(c.attrib["width-ratio"]) for c in b]
            assert abs(sum(ratios) - 1) < 0.002, ratios
            return [[node(k) for k in c] for c in b]
        return b.tag

    tree = [node(b) for b in doc if b.tag in ("img", "grid")]
    assert sorted(b.attrib["src"] for b in doc.findall(".//img")) == [f"assets/{n}.png" for n in colors]
    return tree


def drag(name, lib, previous, moving, target, edge, expected, history=False):
    rects = boxes(previous)
    a, b = rects[moving], rects[target]
    start = ((a[0] + a[2]) / 2, (a[1] + a[3]) / 2)
    end = {"left": (b[0]-4, (b[1]+b[3])/2), "right": (b[2]+4, (b[1]+b[3])/2),
           "top": ((b[0]+b[2])/2, b[1]+2), "bottom": ((b[0]+b[2])/2, b[3]-2)}[edge]
    steps = f"down {start[0]} {start[1]}\nwait 100\ndragto {end[0]} {end[1]}\nwait 200\nshot indicator\nup {end[0]} {end[1]}\n"
    if history:
        steps += "wait 2300\nshot arranged\nkey cmd-z\nwait 2300\nshot undo\nkey cmd-shift-z\n"
    folder = run(name, lib, steps)
    assert layout(folder) == expected, f"{name}: {layout(folder)} != {expected}; {folder}"
    if edge in ("left", "right"):
        im = Image.open(folder / "indicator.png").convert("RGB")
        scale = im.width / 1200
        x = b[0]-5 if edge == "left" else b[2]+5
        # A selected image's 2pt ring insets the solid-color bounds.
        lines = [[im.getpixel((round((x+dx)*scale), round(y*scale))) for y in range(round(b[1]+4), round(b[3]-4))] for dx in range(-3, 4)]
        assert any(sum(blue > 180 and blue > red * 1.5 for red, green, blue in line) > len(line) * 0.7 for line in lines), f"Missing vertical blue indicator: {folder}"
    if history:
        for n, rect in boxes(folder, "undo").items():
            assert all(abs(a-b) < 3 for a, b in zip(rect, rects[n])), f"Undo changed layout: {folder}"
        for n, rect in boxes(folder, "arranged").items():
            assert all(abs(a-b) < 1 for a, b in zip(rect, boxes(folder)[n])), f"Redo changed layout: {folder}"
    print(f"{name}: layout, image properties and indicator verified", flush=True)
    return folder


print(f"Artifacts: {work}", flush=True)
probe = run("probe", source)
right = drag("right", source, probe, "b", "a", "right", [[["a"], ["b"]], "c", "d"], history=True)
third = drag("third", right / "lib", right, "c", "b", "right", [[["a"], ["b"], ["c"]], "d"])
left = drag("left", third / "lib", third, "c", "a", "left", [[["c"], ["a"], ["b"]], "d"])
bottom = drag("bottom", left / "lib", left, "b", "a", "bottom", [[["c"], ["a", "b"]], "d"])
nested = drag("nested", bottom / "lib", bottom, "d", "b", "right", [[["c"], ["a", [["b"], ["d"]]]]])
top = drag("top", source, probe, "d", "a", "top", ["d", "a", "b", "c"])
drag("vertical", source, probe, "a", "b", "bottom", ["b", "a", "c", "d"])
reopened = run("reopened", nested / "lib")
assert layout(reopened) == layout(nested)
narrow = run("narrow", third / "lib", width=850)
r = boxes(narrow, width=850)
assert abs(r["a"][1] - r["b"][1]) < 1 and abs(r["b"][1] - r["c"][1]) < 1, r
assert r["a"][2] < r["b"][0] < r["b"][2] < r["c"][0], r
# A selected image still opens the viewer on a click, and Escape returns to editing.
a = boxes(right)["a"]
x, y = (a[0]+a[2])/2, (a[1]+a[3])/2
viewer = run("viewer", right / "lib", f"click {x} {y}\nwait 200\nclick {x} {y}\nwait 400\nshot viewer\nkey escape\n")
assert layout(viewer) == layout(right)
assert ImageChops.difference(Image.open(viewer / "viewer.png").convert("RGB"), Image.open(viewer / "after.png").convert("RGB")).getbbox(), "Viewer never opened"
print("Image layout: directions, row reorder, nested layouts, history, save/reopen, narrow window and viewer passed", flush=True)
