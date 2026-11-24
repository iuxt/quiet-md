"""Exercise document tabs and save guards through the scripted window."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-tabs-"))
source = work / "source"
(source / ".quiet").mkdir(parents=True)
(source / "Notes").mkdir()
notes = []
for i, title in enumerate(["Tab A", "Tab B", "Tab C"], 1):
    ident = f"8eb491fe-d96f-45c5-a2b5-{i:012d}"
    filename = f"{ident}.xml"
    notes.append(dict(id=ident, title=title, filename=filename, updatedAt=int(time.time()) - i * 120,
                      options=dict(root_order=i)))
    (source / "Notes" / filename).write_text(f"<title>{title}</title>\n" +
        "".join(f"<p>{title} line {n}</p>\n" for n in range(120)))
(source / "quiet.json").write_text(json.dumps(dict(version=2, folders=[], notes=notes, trash=[])))
preferences = json.dumps({
    "appearance": "light", "english": True, "quick_hotkey": False, "onboarding_done": True,
})
(source / ".quiet/preferences.json").write_text(preferences)
script = work / "steps.txt"
script.write_text("""resize 1080 700
move 1040 650
check-tabs prepare
wait 700
key cmd-t
wait 350
check-tabs blank
shot new-tab
click 85 195
check-tabs second
shot two-tabs
click 500 62
wait 300
check-tabs plus
key cmd-t
key cmd-shift-w
check-tabs plus
key ctrl-shift-tab
key ctrl-tab
check-tabs plus
key cmd-shift-w
key ctrl-shift-tab
wait 300
check-tabs undo
key ctrl-tab
check-tabs peer-edit
wait 250
check-tabs peer
check-tabs conflict-prepare
key cmd-t
check-tabs conflict
key cmd-shift-w
check-tabs replace
key cmd-shift-w
check-tabs single
shot single-tab
key cmd-t
wait 300
click 455 208
check-tabs recent
key cmd-shift-w
key cmd-shift-w
check-tabs home
""")
run = work / "run"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
log = (run / "log").read_text()
assert "panicked" not in log, log
for phase in ["prepare", "blank", "second", "plus", "undo", "peer", "conflict-prepare", "conflict", "replace", "single", "recent", "home"]:
    assert f"tabs: {phase} checked" in log, log

empty = work / "empty"
(empty / ".quiet").mkdir(parents=True)
(empty / ".quiet/preferences.json").write_text(preferences)
(empty / "quiet.json").write_text('{"version":2,"folders":[],"notes":[],"trash":[]}')
empty_script = work / "empty-steps.txt"
empty_script.write_text("wait 650\nkey cmd-t\ncheck-tabs empty\n")
empty_run = work / "empty-run"
subprocess.run([sys.executable, str(here / "run.py"), str(empty_script), str(empty), str(empty_run)],
               check=True, timeout=30, env=dict(os.environ, FDL_NOBLINK="1"))
empty_log = (empty_run / "log").read_text()
assert "tabs: empty checked" in empty_log and "panicked" not in empty_log, empty_log
print(f"Tabs checks passed: {run}")
