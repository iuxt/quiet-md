"""Exercise the embedded Excalidraw editor in a temporary Quiet.md library."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-excalidraw-"))
library = work / "source"
note = "33333333-3333-4333-8333-333333333333"
for directory in (".quiet", "notes", "Drawing"):
    (library / directory).mkdir(parents=True)
(library / ".quiet/format.json").write_text('{"format":9}')
(library / ".quiet/preferences.json").write_text(
    '{"theme":"craft","appearance":"light","language":"zh","quick_hotkey":false}'
)
(library / "quiet.json").write_text(json.dumps({
    "version": 2, "folders": [], "trash": [],
    "notes": [{"id": note, "filename": f"{note}.xml", "title": "画板预览", "folderId": None, "options": {"opened": 1}}],
}))
(library / f"notes/{note}.xml").write_text(
    '<title>画板预览</title>\n<p>点击下方画板进入编辑。</p>\n'
    '<drawing path="Drawing/test.excalidraw" width="1200" height="600"/>\n<p></p>\n'
)
elements = [
    dict(type="rectangle", id="draft", x=80, y=130, width=158, height=95,
         fillStyle="solid", backgroundColor="#e5dbff", strokeColor="#6741d9",
         roundness={"type": 3}, seed=2107, customData={"keep": True}),
    dict(type="arrow", id="line", x=250, y=177, width=122, height=0,
         points=[[0, 0], [122, 0]], strokeColor="#6741d9", endArrowhead="arrow", seed=179),
    dict(type="diamond", id="review", x=385, y=106, width=152, height=143,
         fillStyle="solid", backgroundColor="#d0ebff", strokeColor="#1971c2", seed=816),
    dict(type="text", id="label", x=105, y=163, width=113, height=35,
         text="草稿 Draft", fontSize=23, fontFamily=5, lineHeight=1.25, seed=482),
    dict(type="text", id="next-label", x=407, y=165, width=110, height=32,
         text="审阅 Review", fontSize=21, fontFamily=5, lineHeight=1.25, seed=203),
]
original = json.dumps(dict(type="excalidraw", version=2, elements=elements, files={},
                           appState=dict(viewBackgroundColor="#ffffff")))
(library / "Drawing/test.excalidraw").write_text(original)
script = work / "drawing.txt"
script.write_text("""resize 1200 950
wait 1000
move 1150 900
click 650 290
wait 500
shot board
click 412 449
wait 150
shot properties
key r
drag 780 620 965 720
wait 100
shot drawn
key cmd-z
wait 100
shot undo
key cmd-shift-z
wait 100
shot redo
key cmd-s
wait 900
click 106 24
wait 350
shot note
click 650 290
wait 500
shot reopened
click 106 24
wait 200
""")
run = work / "run"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(library), str(run)],
               check=True, timeout=60, env={**os.environ, "FDL_NOBLINK": "1"})
saved = run / "lib"
body = ET.fromstring("<doc>" + (saved / f"notes/{note}.xml").read_text() + "</doc>")
drawing = body.find("drawing")
assert drawing is not None and drawing.attrib["path"] != "Drawing/test.excalidraw", "drawing block was not updated"
assert (saved / "Drawing/test.excalidraw").read_text() == original, "original source was overwritten"
source = json.loads((saved / drawing.attrib["path"]).read_text())
by_id = {element["id"]: element for element in source["elements"]}
assert len(by_id) == len(elements) + 1
assert by_id["draft"]["customData"] == {"keep": True}
assert by_id["line"]["points"] == [[0, 0], [122, 0]]
added, = [e for e in by_id.values() if e["id"] not in {e["id"] for e in elements}]
assert added["type"] == "rectangle" and abs(added["width"] - 185) < 1
shots = {name: (run / f"{name}.png").read_bytes()
         for name in ("board", "properties", "drawn", "undo", "redo", "note", "reopened")}
assert shots["board"] != shots["properties"], "selecting a shape did not open its properties"
assert shots["drawn"] != shots["undo"] and shots["undo"] != shots["redo"], "undo or redo did not redraw the canvas"
assert shots["note"] != shots["reopened"], "the saved block did not reopen the board"
print(f"Drawing opened, edited, saved and reopened. Screenshots: {run}")
