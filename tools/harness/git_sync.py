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
    "appearance": "light", "english": True, "quick_hotkey": False,
}))
script = work / "steps.txt"
script.write_text("""resize 1200 900
wait 1700
move 1150 950
key cmd-,
wait 300
click 120 192
wait 300
shot library-settings
click 120 223
wait 300
click 560 190
type Git UI Tester
click 880 190
type ui@example.invalid
click 880 396
type ui-secret-token
click 600 259
type ssh://git@example.invalid:2222/notes.git
click 600 464
type /tmp/quiet-ui-test-key
click 600 533
type ui-secret-passphrase
click 600 601
type /tmp/quiet-ui-test-ca.crt
shot masked-token
click 470 644
wait 1200
shot enabled
key escape
wait 300
click 110 98
wait 700
click 390 828
key cmd-a
type First UI commit
click 400 872
wait 1500
shot committed
click 970 24
wait 300
shot settings-after-commit
key escape
wait 300
click 110 166
wait 300
click 1104 24
wait 300
shot article-menu
click 1070 410
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
assert (lib / f"Notes/{doc_id}.xml").read_text() == body
repo = lib / ".git"
config = (repo / "config").read_text()
assert "ui-secret-token" not in config and "ui-secret-passphrase" not in config
assert "ssh://git@example.invalid:2222/notes.git" in config
assert "/tmp/quiet-ui-test-key" in config and "/tmp/quiet-ui-test-ca.crt" in config
commit = (repo / "refs/heads/main").read_text().strip()
contents = zlib.decompress((repo / "objects" / commit[:2] / commit[2:]).read_bytes())
assert b"First UI commit" in contents and b"Git UI Tester <ui@example.invalid>" in contents
for name in ("library-settings", "masked-token", "enabled", "committed", "settings-after-commit", "article-menu", "git-history", "returned"):
    assert (run / f"{name}.png").is_file(), name
print(f"Git UI: UUID migration, author, commit, SSH key/CA settings, secret privacy and readonly history verified; screenshots: {run}")
