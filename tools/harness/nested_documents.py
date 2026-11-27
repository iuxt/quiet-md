"""Migrate a version 2 library, edit a former folder and work with its child documents."""
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
(source / ".quiet/preferences.json").write_text(json.dumps({
    "appearance": "light", "english": True, "quick_hotkey": False,
}))
(source / "quiet.json").write_text(json.dumps({"version": 2, "folders": [
    {"id": folder, "name": "Projects", "parentId": None, "options": {"root_order": 1}},
    {"id": child, "name": "Archive", "parentId": folder, "options": {"order": 1}},
], "notes": [{"id": note, "title": "Planning", "filename": f"{note}.xml", "folderId": folder, "options": {"order": 2}}], "trash": []}))
def run_steps(name, source, steps):
    script = work / f"{name}.txt"
    script.write_text(steps)
    run = work / name
    subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
                   check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
    assert "panicked" not in (run / "log").read_text()
    return run

run = run_steps("nested-documents", source, """resize 1200 900
wait 1400
move 1150 850
type Project parent body
wait 750
shot parent
click 400 358
wait 550
shot original-child
down 120 227
wait 100
dragto 120 216
wait 150
dragto 120 196
wait 500
up 120 196
wait 900
shot moved
move 250 196
wait 400
click 248 196
wait 500
click 420 166
type New child
wait 750
shot created
""")
lib = run / "lib"
manifest = json.loads((lib / "quiet.json").read_text())
assert manifest["version"] == 3 and "folders" not in manifest, manifest
assert (lib / ".quiet/manifest-v2.json").read_bytes() == (source / "quiet.json").read_bytes()
notes = {n["id"]: n for n in manifest["notes"]}
assert notes[folder]["title"] == "Projects", notes
assert "Project parent body" in (lib / f"Notes/{folder}.xml").read_text()
assert notes[note]["parentId"] == child and notes[note]["title"] == "Planning", notes
assert (lib / f"Notes/{note}.xml").read_text() == body
assert len(notes) == 4, notes
assert any(n["parentId"] == child and n["title"] == "New child" for n in notes.values()), notes
for n in notes.values():
    assert str(uuid.UUID(n["id"])) == n["id"] and n["filename"] == f'{n["id"]}.xml'
    assert (lib / "Notes" / n["filename"]).is_file()
assert not any((lib / name).exists() for name in ("Projects", "Archive"))
assert (lib / ".git" / "HEAD").read_text().strip() == "ref: refs/heads/main"
assert not (lib / ".git" / "refs" / "heads" / "main").exists()

deleted_run = run_steps("deleted-branch", lib, """resize 1200 900
wait 900
rclick 95 165
wait 350
click 149 530
wait 350
shot confirm-descendants
click 742 490
wait 900
shot deleted-branch
""")
deleted_lib = deleted_run / "lib"
deleted = json.loads((deleted_lib / "quiet.json").read_text())
assert not deleted["notes"] and {t["id"] for t in deleted["trash"]} == set(notes), deleted
assert {t["rootId"] for t in deleted["trash"] if t["id"] != folder} == {folder}, deleted

restored_run = run_steps("restored-branch", deleted_lib, """resize 1200 900
wait 900
click 217 875
wait 350
click 430 109
wait 350
click 1150 24
wait 350
click 735 485
wait 750
shot restored-branch
""")
restored_lib = restored_run / "lib"
restored = json.loads((restored_lib / "quiet.json").read_text())
assert not restored["trash"] and {n["id"]: n for n in restored["notes"]} == notes, restored
for n in notes.values():
    assert (restored_lib / "Notes" / n["filename"]).read_bytes() == (lib / "Notes" / n["filename"]).read_bytes()
print(f"Nested documents migration, editing, moving, creation, deletion and restoration verified: {work}")
