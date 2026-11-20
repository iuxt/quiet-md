"""Local drawing interactions, using only temporary libraries and a fake clipboard."""
import copy
import base64
import struct
import zlib
import json
import math
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
for shape in shapes:
    shape.update(backgroundColor="#ffffff", fillStyle="solid")
shapes[0]["boundElements"] = [dict(id="label", type="text")]
shapes.append(element("text", "label", 120, 125, 60, 25, text="Label", originalText="Label",
                      fontSize=20, fontFamily=5, lineHeight=1.25, textAlign="center",
                      verticalAlign="middle", containerId="a", autoResize=True))
line = [element("line", "line", 100, 100, 400, 200, points=[[0, 0], [200, 0], [400, 200]],
                startBinding=None, endBinding=None, startArrowhead=None, endArrowhead=None,
                polygon=False)]


def source(name, elements, files=None, *, english=True):
    lib = work / (name + "-source")
    for part in [".quiet", "notes", "Drawing"]:
        (lib / part).mkdir(parents=True)
    (lib / ".quiet/format.json").write_text('{"format":9}')
    (lib / ".quiet/preferences.json").write_text(json.dumps(dict(theme="craft", appearance="light", english=english, quick_hotkey=False)))
    (lib / "quiet.json").write_text(json.dumps(dict(version=2, folders=[], trash=[], notes=[dict(
        id=note, title="Drawing regression", filename=note + ".xml", folderId=None, options=dict(opened=1))])))
    (lib / f"notes/{note}.xml").write_text('<title>Drawing regression</title>\n<p>Local drawing</p>\n'
        '<drawing path="Drawing/test.excalidraw" width="1200" height="600"/>\n<p>End</p>\n')
    (lib / "Drawing/test.excalidraw").write_text(json.dumps(dict(type="excalidraw", version=2,
        elements=elements, files=files or {}, appState=dict(viewBackgroundColor="#ffffff"))))
    return lib


def run(name, steps, elements=None, files=None, *, english=True):
    lib = source(name, copy.deepcopy(shapes if elements is None else elements), files, english=english)
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
scene = run("arrow-endpoints", "click 600 499\nwait 100\nshot arrow-selected\ndrag 800 599 900 649\ndrag 400 399 450 349\npointer down 562 424\npointer dragto 587 444\nshot arrow-moving\npointer up 587 444\n", arrow)
assert (live(scene)[0]["x"], live(scene)[0]["y"]) == (175, 70)
assert live(scene)[0]["points"] == [[0, 0], [450, 300]]

scene = run("arrow-first-endpoint-drag", "drag 800 599 850 649\n", arrow)
assert (live(scene)[0]["x"], live(scene)[0]["y"]) == (100, 100)
assert live(scene)[0]["points"] == [[0, 0], [450, 250]]

bound = [element("rectangle", "left", 100, 100, 100, 100), element("rectangle", "right", 600, 100, 100, 100),
         element("arrow", "connection", 200, 150, 400, 0, points=[[0, 0], [400, 0]],
                 startBinding=dict(elementId="left", fixedPoint=[1, 0.5], mode="inside"),
                 endBinding=dict(elementId="right", fixedPoint=[0, 0.5], mode="inside"),
                 startArrowhead=None, endArrowhead="arrow", elbowed=False)]
scene = run("bound-arrow-right-end", "click 600 499\nwait 100\nshot before\npointer down 800 499\npointer dragto 850 649\nshot right-moving\npointer up 850 649\n", bound)
connection = live(scene, "arrow")[0]
assert (connection["x"], connection["y"]) == (200, 150)
assert connection["points"] == [[0, 0], [450, 150]]
assert connection["startBinding"]["elementId"] == "left"

scene = run("bound-arrow-left-end", "click 600 499\nwait 100\nshot before\npointer down 400 499\npointer dragto 450 649\nshot left-moving\npointer up 450 649\n", bound)
connection = live(scene, "arrow")[0]
assert (connection["x"], connection["y"]) == (250, 300)
assert connection["points"] == [[0, 0], [350, -150]]
assert connection["endBinding"]["elementId"] == "right"

legacy_bound = copy.deepcopy(bound)
legacy_bound[-1]["startBinding"] = dict(elementId="left", focus=0, gap=0)
legacy_bound[-1]["endBinding"] = dict(elementId="right", focus=0, gap=0)
scene = run("legacy-bound-arrow-right-end", "click 600 499\ndrag 800 499 850 649\n", legacy_bound)
connection = live(scene, "arrow")[0]
assert (connection["x"], connection["y"]) == (200, 150)
assert connection["points"] == [[0, 0], [450, 150]]

scene = run("bound-arrow-node-end", "click 600 499\nkey cmd-enter\ndrag 800 499 850 649\n", bound)
connection = live(scene, "arrow")[0]
assert (connection["x"], connection["y"]) == (200, 150)
assert connection["points"] == [[0, 0], [450, 150]]

gapped_bound = copy.deepcopy(legacy_bound)
gapped_bound[-1].update(x=210, width=390, points=[[0, 0], [390, 0]])
gapped_bound[-1]["startBinding"]["gap"] = 10
scene = run("gapped-arrow-right-end", "click 600 499\ndrag 800 499 850 649\n", gapped_bound)
connection = live(scene, "arrow")[0]
assert (connection["x"], connection["y"]) == (210, 150)
assert connection["points"] == [[0, 0], [440, 150]]

scene = run("arrow-middle-bend", "click 500 449\nwait 100\nshot controls\ndrag 600 499 600 399\nshot bent\ndrag 600 399 650 349\n", arrow)
connection = live(scene, "arrow")[0]
assert connection["points"] == [[0, 0], [250, -50], [400, 200]]
assert (connection["x"], connection["y"]) == (100, 100) and connection["roundness"] is not None

curved_arrow = copy.deepcopy(live(scene))
scene = run("curve-click", "click 600 364\nkey shift-down\n", curved_arrow)
connection = live(scene, "arrow")[0]
assert (connection["x"], connection["y"]) == (100, 105)
assert connection["points"] == [[0, 0], [250, -50], [400, 200]]

scene = run("arrow-corner-resize", "click 500 449\nwait 100\ndrag 812 611 912 711\n", arrow)
connection = live(scene, "arrow")[0]
assert (connection["x"], connection["y"]) == (100, 100)
assert connection["points"] == [[0, 0], [500, 300]]

scene = run("arrow-rotate-resize", "click 500 449\nwait 100\ndrag 600 371 728 499\nshot rotated\ndrag 488 711 438 811\n", arrow)
connection = live(scene, "arrow")[0]
assert abs(connection["angle"] - math.pi / 2) < 0.000001
assert all(abs(a - b) < 0.000001 for a, b in zip(connection["points"][-1], [500, 250]))
pivot = (connection["x"] + connection["width"] / 2, connection["y"] + connection["height"] / 2)
a = connection["angle"]
first = (pivot[0] + math.cos(a) * (connection["x"] - pivot[0]) - math.sin(a) * (connection["y"] - pivot[1]),
         pivot[1] + math.sin(a) * (connection["x"] - pivot[0]) + math.cos(a) * (connection["y"] - pivot[1]))
assert math.dist(first, [400, 0]) < 0.000001

scene = run("arrow-rotate-45", "click 500 449\nwait 100\npointer drag 600 371 690 409 shift\n", arrow)
assert abs(live(scene, "arrow")[0]["angle"] - math.pi / 4) < 0.000001

scene = run("shift-select", "click 350 399\npointer click 600 499 shift\nkey shift-down\n")
assert [(e["x"], e["y"]) for e in live(scene, "rectangle")] == [(100, 105), (350, 205), (600, 300)]

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

scene = run("duplicate-points", """shot pan-hint
click 550 399
wait 100
shot line-hint
key cmd-enter
wait 100
shot points-hint
click 600 399
wait 100
shot selected-point-hint
key cmd-d
key shift-right
key cmd-z
key cmd-z
key cmd-shift-z
key cmd-shift-z
""", line, english=False)
assert len(live(scene)) == 1
assert live(scene)[0]["points"] == [[0, 0], [200, 0], [305, 100], [400, 200]]

scene = run("polyline", "key l\nclick 400 300\nclick 600 300\nclick 650 450\nkey enter\n", [])
assert live(scene, "line")[0]["points"] == [[0, 0], [200, 0], [250, 150]]

scene = run("multiline", """dclick 600 350
type Alpha
key enter
type Beta
key cmd-enter
key v
key cmd-f
wait 100
type Beta
wait 100
shot find-hint
key escape
key shift-right
""", [])
assert live(scene, "text")[0]["originalText"] == "Alpha\nBeta"
assert live(scene, "text")[0]["x"] == 605

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
styled[0].update(backgroundColor="#e03131", fillStyle="solid")
scene = run("styles", "click 350 399\nkey cmd-alt-c\nclick 600 499\nkey cmd-alt-v\n", styled)
assert live(scene, "rectangle")[1]["strokeColor"] == "#e03131"

scene = run("deep-selection", "click 350 399\npointer click 850 599 cmd\nkey shift-down\n",
            [dict(e, groupIds=["group"]) for e in shapes])
assert [(e["x"], e["y"]) for e in live(scene, "rectangle")] == [(100, 100), (350, 200), (600, 305)]

scene = run("object-snapping", "click 350 399\nkey alt-s\ndrag 350 399 596 496\n")
assert (live(scene, "rectangle")[0]["x"], live(scene, "rectangle")[0]["y"]) == (350, 200)

scene = run("convert-shape", "click 350 399\nkey tab\n")
assert len(live(scene, "diamond")) == 1 and live(scene, "text")[0]["containerId"] == "a"

scene = run("flowchart", "click 600 499\nkey cmd-right\nkey alt-left\nkey shift-up\n")
assert len(live(scene, "rectangle")) == 4 and len(live(scene, "arrow")) == 1
arrow = live(scene, "arrow")[0]
assert arrow["startBinding"]["elementId"] == "b" and arrow["endBinding"]["elementId"] == live(scene, "rectangle")[-1]["id"]
assert live(scene, "rectangle")[1]["y"] == 195

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
assert live(scene, "line")[0]["polygon"] and live(scene, "line")[0]["backgroundColor"] == "#b2f2bb"

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
assert len(library["libraryItems"]) == 1
assert len(json.loads(library_export.read_text())["libraryItems"]) == 1

export_path = work / "drawing.png"
scene = run("export-reimport", f"key cmd-shift-e\nwait 150\nclick 100 752\npick {export_path}\nclick 100 840\nwait 500\npick {export_path}\nkey cmd-o\nwait 500\n")
assert export_path.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
assert [e["id"] for e in live(scene)] == [e["id"] for e in shapes]

# Compatibility regressions use the same save/import and clipboard paths as a real note.
scene = run("readonly-undo", "click 600 499\nkey shift-down\nkey alt-r\nkey cmd-z\nclick 163 923\n")
assert live(scene, "rectangle")[1]["y"] == 205

scene = run("duplicate-extensions", "click 350 399\nkey cmd-d\n", [dict(shapes[0], futureShape={"flag": 42}), *shapes[1:]])
assert [e.get("futureShape") for e in live(scene, "rectangle")].count({"flag": 42}) == 2

hollow = copy.deepcopy(shapes)
hollow[1]["backgroundColor"] = "transparent"
scene = run("hollow-selection", "click 600 499\nkey delete\n", hollow)
assert len(live(scene, "rectangle")) == 3

scene = run("wrap-frame", "key cmd-a\nkey w\n")
frame = live(scene, "frame")[0]
assert all(e["frameId"] == frame["id"] for e in live(scene) if e != frame)
scene = run("unwrap-frame", "key cmd-a\nkey w\nkey w\n")
assert not live(scene, "frame")

long_text = work / "long-text.txt"
long_text.write_text("画板示例文字。" * 450)
scene = run("long-unicode-paste", f"clip text {long_text}\nmove 550 450\nkey cmd-v\nkey cmd-enter\n", [])
assert live(scene, "text")[0]["originalText"] == long_text.read_text()

fixture = here / "fixtures/excalidraw/smiley_embedded_v2.png"
scene = run("official-editable-png", f"pick {fixture}\nkey cmd-o\nwait 450\n")
assert live(scene, "text") and all(e["id"] != "a" for e in live(scene))

scene = run("mermaid-palette", "key cmd-shift-p\ntype Mermaid\nkey enter\ntype flowchart LR\nkey enter\ntype A --> B\nkey cmd-enter\n", [])
assert len(live(scene, "arrow")) == 1 and len(live(scene, "text")) == 2

scene = run("dirty-import-cancel", f"click 600 499\nkey shift-down\npick {fixture}\nkey cmd-o\nwait 450\nkey escape\n")
assert live(scene, "rectangle")[1]["y"] == 205

svg = work / "local-image.svg"
svg.write_text('<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40" fill="#ff0000"/></svg>')
scene = run("drop-svg", f"filedrag 500 420 {svg}\nfiledrop 500 420\nwait 400\n", [])
img = live(scene, "image")[0]
assert (img["width"], img["height"]) == (80, 40)
assert scene["files"][img["fileId"]]["mimeType"] == "image/svg+xml"

scene = run("recognize-line", "key x\npointer down 411 410\npointer dragto 482 450\npointer dragto 553 490\npointer up 553 490\n", [])
assert len(live(scene, "line")) == 1 and len(live(scene, "line")[0]["points"]) == 2

scene = run("grid-persistence", "key shift-g\n")
assert scene["appState"]["gridModeEnabled"] and scene["appState"]["gridSize"] == 20

spreadsheet = work / "spreadsheet.tsv"
spreadsheet.write_text("Month\tCount\nMarch\t5\nApril\t7\nMay\t3")
scene = run("chart-paste", f"clip text {spreadsheet}\nmove 600 450\nkey cmd-v\nwait 200\nclick 100 779\n", [])
assert len(live(scene, "rectangle")) == 3

scene = run("freehand-dot-and-pressure", """key p
pointer click 420 410
pointer down 440 460 pressure=0.25
pointer dragto 500 468 pressure=0.75
pointer up 560 472 pressure=0
""", [])
dot, stroke = live(scene, "freedraw")
assert len(dot["points"]) == 2 and dot["simulatePressure"] and dot["pressures"] == []
assert dot["strokeOptions"]["streamline"] == 0.5
assert stroke["pressures"] == [0.25, 0.75, 0.0] and not stroke["simulatePressure"]
assert stroke["strokeOptions"]["streamline"] == 0.2

scene = run("detach-frame-contents", """key f
drag 280 339 930 669
key cmd-shift-p
type Remove all elements from frame
key enter
key r
drag 955 710 1030 755
""")
assert len(live(scene, "frame")) == 1
assert all(e["frameId"] is None for e in live(scene, "rectangle"))

scene = run("mermaid-image-fallback", """key cmd-shift-p
type Mermaid
key enter
type sequenceDiagram
key enter
type Alice->>Bob: Hello
key cmd-enter
wait 500
""", [])
img = live(scene, "image")[0]
assert scene["files"][img["fileId"]]["mimeType"] == "image/svg+xml"

scene = run("explicit-shape-switch", """click 600 499
wait 180
key cmd-shift-p
wait 180
type Shape switch
wait 180
key enter
wait 180
pointer click 100 872
""")
assert next(e for e in live(scene) if e["id"] == "b")["type"] == "ellipse"

frame_png = work / "frame-only.png"
scene = run("frame-export-default-selection", f"""key f
drag 280 339 930 669
key cmd-shift-e
wait 200
pick {frame_png}
click 100 840
wait 400
""")
frame = live(scene, "frame")[0]
width, height = struct.unpack(">II", frame_png.read_bytes()[16:24])
assert (width, height) == (int(2 * frame["width"]), int(2 * frame["height"]))

print("Drawing regression passed:", work)
