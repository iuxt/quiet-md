"""Git change list, readable/source diffs and whole-library commit in a throwaway library.

Build with --release --features devtools. Reuses the Git settings regression
for a real initial commit; no network, system Git or user library is used.
"""
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile

here = Path(__file__).resolve().parent
seed = runpy.run_path(str(here / "git_sync.py"))
work = Path(tempfile.mkdtemp(prefix="quiet-git-changes-"))
source = work / "source"
shutil.copytree(seed["lib"], source)
doc_id = seed["doc_id"]
body = "<title>Git regression</title>\n<p>Review this updated body.</p>\n<p>A new paragraph.</p>\n"
(source / f"notes/{doc_id}.xml").write_text(body)
metadata = json.loads((source / "quiet.json").read_text())
new = dict(metadata["notes"][0])
new.update(id="bbbbbbbb-0000-4000-8000-000000000002", filename="bbbbbbbb-0000-4000-8000-000000000002.md", title="发布检查清单")
metadata["notes"].append(new)
(source / "quiet.json").write_text(json.dumps(metadata, ensure_ascii=False, indent=2))
(source / "notes" / new["filename"]).write_text("# 发布检查清单\n\n- 检查文档和资源\n- 验证 Git 文件对比\n")
(source / ".assets").mkdir(exist_ok=True)
(source / ".assets/attachment.bin").write_bytes(b"\0binary attachment")
prefs = source / ".quiet/preferences.json"
preferences = json.loads(prefs.read_text())
preferences.update(english=False, language="zh")
prefs.write_text(json.dumps(preferences))
head_before = (source / ".git/refs/heads/main").read_text()
script = work / "steps.txt"
script.write_text("""resize 1280 860
wait 1400
move 1220 800
click 110 98
wait 900
move 1220 800
wait 400
shot text-diff
click 1205 150
move 1220 800
wait 400
shot source-diff
click 380 295
move 1220 800
wait 500
shot added-markdown
click 380 350
move 1220 800
wait 500
shot metadata-diff
click 380 180
move 1220 800
wait 500
shot binary-change
click 380 235
move 1220 800
wait 500
type SHOULD_NOT_WRITE
resize 780 640
wait 400
shot narrow-diff
resize 1280 860
wait 300
click 390 788
type Review library changes
click 400 832
move 1220 800
wait 1200
shot clean-worktree
""")
run = work / "native-git-changes"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
lib = run / "lib"
assert (lib / f"notes/{doc_id}.xml").read_text() == body, "Diff must stay read-only"
assert (lib / ".git/refs/heads/main").read_text() != head_before, "Panel commit did not advance HEAD"
for name in ("text-diff", "source-diff", "added-markdown", "metadata-diff", "binary-change", "narrow-diff", "clean-worktree"):
    assert (run / f"{name}.png").is_file(), name
assert "panicked" not in (run / "log").read_text()
print(f"Git changes: readonly diff, text/source, Markdown, metadata, binary, narrow window and commit verified; screenshots: {run}")
