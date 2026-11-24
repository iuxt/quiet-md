"""Exercise virtual folder moves, renames and creation in a temporary library."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-virtual-ui-"))
source = work / "source"
(source / ".quiet").mkdir(parents=True)
(source / "Notes").mkdir()
folder = "11111111-1111-4111-8111-111111111111"
child = "22222222-2222-4222-8222-222222222222"
note = "33333333-3333-4333-8333-333333333333"
body = "<title>Stored body title</title>\n<p>Original body.</p>\n"
(source / f"Notes/{note}.xml").write_text(body)
(source / ".quiet/format.json").write_text('{"format":11}')
(source / ".quiet/preferences.json").write_text(json.dumps({
    "appearance": "light", "english": True, "quick_hotkey": False,
}))
(source / "quiet.json").write_text(json.dumps({"version": 2, "folders": [
    {"id": folder, "name": "Projects", "parentId": None, "options": {"root_order": 1}},
    {"id": child, "name": "Archive", "parentId": folder, "options": {"order": 1}},
], "notes": [{"id": note, "title": "Planning", "filename": f"{note}.xml", "folderId": folder, "options": {"order": 2}}], "trash": []}))
script = work / "steps.txt"
script.write_text("""resize 1200 1000
wait 1400
move 1150 950
click 100 227
wait 300
shot metadata-title
down 120 227
wait 100
dragto 120 216
wait 150
dragto 120 196
wait 500
up 120 196
wait 900
shot moved
move 200 164
wait 250
click 200 164
wait 200
click 260 263
wait 150
type Renamed Projects
key enter
wait 700
shot renamed
click 115 196
wait 250
key cmd-n
wait 250
type New virtual note
wait 700
shot created
""")
run = work / "native-virtual-folders"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
lib = run / "lib"
manifest = json.loads((lib / "quiet.json").read_text())
folders = {f["id"]: f for f in manifest["folders"]}
notes = {n["id"]: n for n in manifest["notes"]}
assert folders[folder]["name"] == "Renamed Projects", folders
assert notes[note]["folderId"] == child and notes[note]["title"] == "Planning", notes
assert (lib / f"Notes/{note}.xml").read_text() == body
assert len(notes) == 2, notes
for n in notes.values():
    assert str(uuid.UUID(n["id"])) == n["id"] and n["filename"] == f'{n["id"]}.xml'
    assert n["folderId"] == child
    assert (lib / "Notes" / n["filename"]).is_file()
assert not any((lib / name).exists() for name in ("Projects", "Renamed Projects", "Archive"))
assert (lib / ".git" / "HEAD").read_text().strip() == "ref: refs/heads/main"
assert not (lib / ".git" / "refs" / "heads" / "main").exists()
print(f"Virtual folders UI: JSON titles, move, rename and UUID creation verified; screenshots: {run}")
