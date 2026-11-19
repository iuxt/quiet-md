"""Local drawing interactions, using only temporary libraries and a fake clipboard."""
import copy
import base64
import struct
import zlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-drawing-"))
note = "33333333-3333-4333-8333-333333333333"


def element(kind, identity, x, y, width, height, **fields):
    return dict(type=kind, id=identity, x=x, y=y, width=width, height=height,
                strokeColor="#1e1e1e", backgroundColor="transparent", fillStyle="hachure",
                strokeWidth=2, strokeStyle="solid", roughness=0, opacity=100, angle=0,
                seed=1, version=1, versionNonce=0, isDeleted=False, groupIds=[], locked=False,
                roundness=None, index=None, frameId=None, boundElements=None, updated=0,
                created=None, link=None, **fields)


shapes = [element("rectangle", "a", 100, 100, 100, 80),
          element("rectangle", "b", 350, 200, 100, 80),
          element("rectangle", "c", 600, 300, 100, 80)]
shapes[0]["boundElements"] = [dict(id="label", type="text")]
shapes.append(element("text", "label", 120, 125, 60, 25, text="Label", originalText="Label",
                      fontSize=20, fontFamily=5, lineHeight=1.25, textAlign="center",
                      verticalAlign="middle", containerId="a", autoResize=True))
line = [element("line", "line", 100, 100, 400, 200, points=[[0, 0], [200, 0], [400, 200]],
                startBinding=None, endBinding=None, startArrowhead=None, endArrowhead=None,
                polygon=False)]


def source(name, elements, files=None):
    lib = work / (name + "-source")
    for part in [".quiet", "notes", "Drawing"]:
        (lib / part).mkdir(parents=True)
    (lib / ".quiet/format.json").write_text('{"format":9}')
    (lib / ".quiet/preferences.json").write_text(json.dumps(dict(theme="craft", appearance="light", english=True, quick_hotkey=False)))
    (lib / "quiet.json").write_text(json.dumps(dict(version=2, folders=[], trash=[], notes=[dict(
        id=note, title="Drawing regression", filename=note + ".xml", folderId=None, options=dict(opened=1))])))
    (lib / f"notes/{note}.xml").write_text('<title>Drawing regression</title>\n<p>Local drawing</p>\n'
        '<drawing path="Drawing/test.excalidraw" width="1200" height="600"/>\n<p>End</p>\n')
    (lib / "Drawing/test.excalidraw").write_text(json.dumps(dict(type="excalidraw", version=2,
        elements=elements, files=files or {}, appState=dict(viewBackgroundColor="#ffffff"))))
    return lib


def run(name, steps, elements=None, files=None):
    lib = source(name, copy.deepcopy(shapes if elements is None else elements), files)
    script = work / (name + ".txt")
    script.write_text("resize 1200 950\nwait 1200\nmove 1150 900\nclick 650 290\nwait 400\n"
                      + steps + "\nwait 150\nshot result\nkey cmd-s\nwait 900\nclick 108 24\nwait 350\n")
    folder = work / name
    subprocess.run([sys.executable, str(here / "run.py"), str(script), str(lib), str(folder)],
                   check=True, timeout=90, env=dict(os.environ, FDL_NOBLINK="1"))
    body = ET.fromstring("<doc>" + (folder / "lib" / f"notes/{note}.xml").read_text() + "</doc>")
    drawing = body.find("drawing")
    assert drawing is not None, name
    scene = json.loads((folder / "lib" / drawing.attrib["path"]).read_text())
    ids = [e["id"] for e in scene["elements"]]
    assert len(ids) == len(set(ids)), (name, "duplicate ids")
    print("Exercised interaction:", name, flush=True)
    return scene


def live(scene, kind=None):
    return [e for e in scene["elements"] if not e["isDeleted"] and (kind is None or e["type"] == kind)]


arrow = [element("arrow", "arrow", 100, 100, 400, 200, points=[[0, 0], [400, 200]],
                 startBinding=None, endBinding=None, startArrowhead=None, endArrowhead="arrow", elbowed=False)]
scene = run("arrow-endpoints", "click 600 499\nwait 100\nshot arrow-selected\ndrag 800 599 900 649\ndrag 400 399 450 349\npointer down 675 499\npointer dragto 700 519\nshot arrow-moving\npointer up 700 519\n", arrow)
assert (live(scene)[0]["x"], live(scene)[0]["y"]) == (175, 70)
assert live(scene)[0]["points"] == [[0, 0], [450, 300]]

scene = run("shift-select", "click 350 399\npointer click 600 499 shift\nkey shift-down\n")
assert [(e["x"], e["y"]) for e in live(scene, "rectangle")] == [(100, 110), (350, 210), (600, 300)]

scene = run("box-group-move", "drag 290 349 910 649\nkey cmd-g\nclick 1100 800\nclick 350 399\ndrag 350 399 390 429\n")
assert [(e["x"], e["y"]) for e in live(scene, "rectangle")] == [(140, 130), (390, 230), (640, 330)]
assert len({tuple(e["groupIds"]) for e in live(scene)}) == 1
assert live(scene)[0]["groupIds"]

scene = run("duplicate", "key cmd-a\nkey cmd-g\nkey cmd-d\n")
assert len(live(scene)) == 8
labels = live(scene, "text")
assert len({e["containerId"] for e in labels}) == 2
assert len({tuple(e["groupIds"]) for e in live(scene)}) == 2

scene = run("clipboard", "click 350 399\npointer click 600 499 shift\nclip clear\nkey cmd-c\nmove 900 750\nkey cmd-v\n")
assert len(live(scene, "rectangle")) == 5 and len(live(scene, "text")) == 2
assert len({e["containerId"] for e in live(scene, "text")}) == 2

scene = run("alt-drag", "click 350 399\npointer drag 350 399 450 499 alt\n")
assert len(live(scene, "rectangle")) == 4
assert (live(scene, "rectangle")[-1]["x"], live(scene, "rectangle")[-1]["y"]) == (200, 200)

scene = run("align", "key cmd-a\nkey cmd-shift-left\n")
assert [e["x"] for e in live(scene, "rectangle")] == [100, 100, 100]

scene = run("group-resize", "key cmd-a\ndrag 900 639 1000 739\n")
assert all(abs(e["width"] - 100 * 7 / 6) < 0.001 for e in live(scene, "rectangle"))

scene = run("group-rotate", "key cmd-a\ndrag 600 342 910 499\n")
assert all(abs(e["angle"] - 1.57079632679) < 0.001 for e in live(scene, "rectangle"))

scene = run("nodes", "click 500 399\nkey cmd-enter\ndrag 500 399 500 459\nkey backspace\nkey cmd-z\n", line)
assert live(scene)[0]["points"] == [[0, 0], [100, 60], [200, 0], [400, 200]]

scene = run("polyline", "key l\nclick 400 300\nclick 600 300\nclick 650 450\nkey enter\n", [])
assert live(scene, "line")[0]["points"] == [[0, 0], [200, 0], [250, 150]]

scene = run("multiline", "dclick 600 350\ntype Alpha\nkey enter\ntype Beta\nkey cmd-enter\n", [])
assert live(scene, "text")[0]["originalText"] == "Alpha\nBeta"

scene = run("frame", "key f\ndrag 280 339 930 669\nclick 1100 800\nclick 280 500\ndrag 280 500 300 520\n")
frame = live(scene, "frame")[0]
assert all(e["frameId"] == frame["id"] for e in live(scene, "rectangle"))
assert [(e["x"], e["y"]) for e in live(scene, "rectangle")] == [(120, 120), (370, 220), (620, 320)]

scene = run("history", "key cmd-a\nkey delete\nkey cmd-z\nkey cmd-shift-z\nkey cmd-z\n")
assert len(live(scene, "rectangle")) == 3 and len(live(scene, "text")) == 1

scene = run("lock", "key cmd-a\nkey cmd-shift-l\nclick 350 399\nkey delete\n")
assert len(live(scene)) == 4 and all(e["locked"] for e in live(scene))

styled = copy.deepcopy(shapes)
styled[0]["strokeColor"] = "#e03131"
scene = run("styles", "click 350 399\nkey cmd-alt-c\nclick 600 499\nkey cmd-alt-v\n", styled)
assert live(scene, "rectangle")[1]["strokeColor"] == "#e03131"

scene = run("deep-selection", "click 350 399\npointer click 850 599 cmd\nkey shift-down\n",
            [dict(e, groupIds=["group"]) for e in shapes])
assert [(e["x"], e["y"]) for e in live(scene, "rectangle")] == [(100, 100), (350, 200), (600, 310)]

scene = run("object-snapping", "click 350 399\nkey alt-s\ndrag 350 399 596 496\n")
assert (live(scene, "rectangle")[0]["x"], live(scene, "rectangle")[0]["y"]) == (350, 200)

scene = run("convert-shape", "click 350 399\nkey tab\n")
assert len(live(scene, "diamond")) == 1 and live(scene, "text")[0]["containerId"] == "a"

scene = run("flowchart", "click 600 499\nkey cmd-right\nkey alt-left\nkey shift-up\n")
assert len(live(scene, "rectangle")) == 4 and len(live(scene, "arrow")) == 1
arrow = live(scene, "arrow")[0]
assert arrow["startBinding"]["elementId"] == "b" and arrow["endBinding"]["elementId"] == live(scene, "rectangle")[-1]["id"]
assert live(scene, "rectangle")[1]["y"] == 190

scene = run("unbind-text", "click 350 399\nkey w\n")
assert live(scene, "text")[0]["containerId"] is None
assert not any(e["type"] == "text" for e in (live(scene, "rectangle")[0]["boundElements"] or []))

scene = run("bind-text", "click 350 399\nkey w\nkey w\n")
assert live(scene, "text")[0]["containerId"] == "a"

scene = run("remove-text", "click 350 399\nkey enter\nkey backspace\nkey cmd-enter\n")
assert not live(scene, "text") and len(live(scene, "rectangle")) == 3

scene = run("copy-png", "key cmd-a\nclip clear\nkey alt-shift-c\nwait 700\nmove 900 750\nkey cmd-v\nwait 500\n")
assert len(live(scene, "image")) == 1 and len(scene["files"]) == 1


def png(width, height):
    def chunk(tag, value):
        return struct.pack(">I", len(value)) + tag + value + struct.pack(">I", zlib.crc32(tag + value))
    pixels = b"".join(b"\0" + b"\xff\0\0\xff" * (width // 2) + b"\0\0\xff\xff" * (width - width // 2) for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")


image_bytes = png(200, 100)
image_path = work / "clipboard.png"
image_path.write_bytes(image_bytes)
scene = run("paste-image", f"clip clear\nclip image {image_path}\nmove 600 450\nkey cmd-v\nwait 500\n", [])
assert len(live(scene, "image")) == 1 and len(scene["files"]) == 1

image_scene = [element("image", "image", 100, 100, 400, 200, fileId="pixels", scale=[1, 1], status="saved", crop=None)]
files = dict(pixels=dict(id="pixels", mimeType="image/png", dataURL="data:image/png;base64," + base64.b64encode(image_bytes).decode(), created=0))
scene = run("crop-and-flip", "click 600 499\nkey enter\ndrag 400 399 500 449\nkey escape\nkey shift-h\n", image_scene, files)
image = live(scene, "image")[0]
assert (image["x"], image["y"], image["width"], image["height"]) == (200, 150, 300, 150)
assert image["scale"] == [-1, 1] and image["crop"] == dict(x=50, y=25, width=150, height=75, naturalWidth=200, naturalHeight=100)

scene = run("eyedropper", "click 600 499\nkey i\nclick 310 369\n", styled)
assert live(scene, "rectangle")[1]["strokeColor"] == "#e03131"

scene = run("bucket-fill", "key b\nclick 600 499\n")
assert live(scene, "rectangle")[1]["backgroundColor"] == "#1e1e1e"

scene = run("laser", "key k\ndrag 400 300 900 650\nwait 1100\n")
assert live(scene) == shapes

scene = run("centered-shape", "key r\npointer drag 600 450 700 500 alt shift\n", [])
rect = live(scene, "rectangle")[0]
assert (rect["x"], rect["y"], rect["width"], rect["height"]) == (500, 302, 200, 200)

scene = run("precise-width", "click 600 499\nwait 150\nshot properties\nclick 90 181\ntype 160\nkey enter\n")
assert live(scene, "rectangle")[1]["width"] == 160

library_export = work / "library.excalidrawlib"
scene = run("local-library", f"key cmd-a\nkey shift-l\nshot library-before\nclick 100 792\nshot library-after\npick {library_export}\nclick 100 778\nwait 200\nclick 100 842\nkey shift-l\nwait 150\npick {library_export}\nclick 100 740\nwait 300\n")
assert len(live(scene, "rectangle")) == 6 and len(live(scene, "text")) == 2
library = json.loads((work / "local-library/lib/.quiet/drawing-library.excalidrawlib").read_text())
assert len(library["libraryItems"]) == 2
assert len(json.loads(library_export.read_text())["libraryItems"]) == 1

export_path = work / "drawing.png"
scene = run("export-reimport", f"key cmd-shift-e\nwait 150\npick {export_path}\nclick 100 840\nwait 500\npick {export_path}\nkey cmd-o\nwait 500\n")
assert export_path.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
assert [e["id"] for e in live(scene)] == [e["id"] for e in shapes]

print("Drawing regression passed:", work)
