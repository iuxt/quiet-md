"""Regression checks for preview synchronization, imports and delete navigation.

Build with --release --features devtools. All files and libraries are temporary;
windows remain off screen and no real clipboard or system dialogs are used.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-document-safety-"))
source = work / "source"
(source / ".quiet").mkdir(parents=True)
(source / ".quiet/format.json").write_text('{"format":9}')
(source / "quiet.json").write_text('{"version":2,"folders":[],"notes":[],"trash":[]}')
(source / ".quiet/preferences.json").write_text(json.dumps({
    "appearance": "light", "english": True, "quick_hotkey": False, "onboarding_done": True,
}))
incoming = work / "incoming"
incoming.mkdir()
xml = incoming / "Imported.xml"
xml.write_text('<title>Imported safety</title>\n<p>Imported body.</p>\n')
for name in ["FolderA", "FolderB", "FolderC", "FolderD"]:
    folder = incoming / name
    folder.mkdir()
    (folder / "Note.md").write_text(f"{name} body\n")
for name in ["Loose", "SidebarLoose"]:
    (incoming / f"{name}.md").write_text(f"{name} body\n")

script = work / "steps.txt"
script.write_text(f"""resize 1200 900
wait 1500
move 1150 850
check-document-safety preview
click 1144 24
wait 250
pick {xml}
click 1040 125
wait 1600
check-document-safety imported
shot imported
check-document-safety delete
wait 1600
check-document-safety deleted
shot deleted
click 75 66
wait 400
filedrag 700 300 {incoming / 'FolderA'}|{incoming / 'FolderB'}|{incoming / 'Loose.md'}
wait 250
filedrop 700 300
wait 1800
shot home-batch
filedrag 110 105 {incoming / 'FolderC'}|{incoming / 'FolderD'}|{incoming / 'SidebarLoose.md'}
wait 250
filedrop 110 105
wait 250
shot sidebar-confirm
click 740 485
wait 1800
shot sidebar-batch
""")
run = work / "run"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
log = (run / "log").read_text()
assert "panicked" not in log, log
for phase in ["preview", "imported", "delete", "deleted"]:
    assert f"document-safety: {phase} checked" in log, log
manifest = json.loads((run / "lib/quiet.json").read_text())
assert {f["name"] for f in manifest["folders"]} == {"FolderA", "FolderB", "FolderC", "FolderD"}, manifest
notes = manifest["notes"]
assert {n["title"] for n in notes} == {"Preview safety", "Loose", "SidebarLoose", "Note"}, notes
assert len(notes) == 7, notes
assert any(t["title"] == "Imported safety" for t in manifest["trash"]), manifest
preview = next(n for n in notes if n["title"] == "Preview safety")
assert "Original from B from A" in (run / "lib/notes" / preview["filename"]).read_text()
print(f"Document safety checks passed: {run}")
