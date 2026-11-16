"""Enable Git, commit and view article history through the native UI.

Build with --release --features devtools. Uses only a temporary library;
no Git executable, network, real clipboard or installed application.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zlib

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-git-ui-"))
source = work / "source"
(source / ".quiet").mkdir(parents=True)
doc_id = "aaaaaaaa-0000-4000-8000-000000000001"
body = "<title>Git regression</title>\n<p>Keep this body.</p>\n"
(source / "Git regression.xml").write_text(body)
(source / ".quiet/format.json").write_text('{"format":4}')
(source / ".quiet/index.json").write_text(json.dumps({"Git regression.xml": {"id": doc_id, "created": 1, "modified": 1}}))
(source / ".quiet/preferences.json").write_text(json.dumps({
    "appearance": "light", "english": True, "seen_version": "0.1.12", "auto_update": False, "quick_hotkey": False,
}))
script = work / "steps.txt"
script.write_text("""resize 1200 1300
wait 1700
move 1150 950
key cmd-,
wait 300
click 120 192
wait 300
click 560 411
type Git UI Tester
click 880 411
type ui@example.invalid
click 880 617
type ui-secret-token
click 600 480
type ssh://git@example.invalid:2222/notes.git
click 600 685
type /tmp/quiet-ui-test-key
click 600 754
type ui-secret-passphrase
click 600 822
type /tmp/quiet-ui-test-ca.crt
shot masked-token
click 470 858
wait 1200
shot enabled
click 580 926
type First UI commit
click 525 968
wait 1500
shot committed
click 700 1134
wait 700
shot git-history
type SHOULD_NOT_WRITE
wait 200
key escape
key escape
wait 300
shot returned
""")
run = work / "native-git-integration"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
lib = run / "lib"
metadata = json.loads((lib / "quiet.json").read_text())
assert metadata["version"] == 2 and len(metadata["notes"]) == 1, metadata
assert metadata["notes"][0]["filename"] == f"{doc_id}.xml"
assert metadata["notes"][0]["title"] == "Git regression"
assert not (lib / "Git regression.xml").exists()
backups = list((lib / ".quiet").glob("before-virtual-6-*"))
assert len(backups) == 1 and (backups[0] / "Git regression.xml").read_text() == body
assert (lib / f"notes/{doc_id}.xml").read_text() == body
repo = lib / ".git"
config = (repo / "config").read_text()
assert "ui-secret-token" not in config and "ui-secret-passphrase" not in config
assert "ssh://git@example.invalid:2222/notes.git" in config
assert "/tmp/quiet-ui-test-key" in config and "/tmp/quiet-ui-test-ca.crt" in config
commit = (repo / "refs/heads/main").read_text().strip()
contents = zlib.decompress((repo / "objects" / commit[:2] / commit[2:]).read_bytes())
assert b"First UI commit" in contents and b"Git UI Tester <ui@example.invalid>" in contents
for name in ("masked-token", "enabled", "committed", "git-history", "returned"):
    assert (run / f"{name}.png").is_file(), name
print(f"Git UI: UUID migration, author, commit, SSH key/CA settings, secret privacy and readonly history verified; screenshots: {run}")
