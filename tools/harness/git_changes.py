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
import zlib

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
(source / "Files").mkdir(exist_ok=True)
(source / "Files/attachment.bin").write_bytes(b"\0binary attachment")
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
click 380 347
move 1220 800
wait 500
shot added-markdown
click 380 427
move 1220 800
wait 500
shot metadata-diff
click 380 187
move 1220 800
wait 500
shot binary-change
click 380 267
move 1220 800
wait 500
type SHOULD_NOT_WRITE
resize 780 640
wait 400
shot narrow-diff
resize 1280 860
wait 300
click 390 788
key cmd-a
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

def replay(name, steps):
    script = work / f"{name}.txt"
    script.write_text("resize 1280 860\nwait 1400\nmove 1220 800\nclick 110 98\nwait 900\n" + steps)
    target = work / name
    subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(target)],
                   check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
    assert "panicked" not in (target / "log").read_text()
    return target

def object_bytes(repo, oid):
    return zlib.decompress((repo / "objects" / oid[:2] / oid[2:]).read_bytes()).split(b"\0", 1)[1]

def committed_files(repo):
    commit = (repo / "refs/heads/main").read_text().strip()
    tree = object_bytes(repo, commit).splitlines()[0].split()[1].decode()
    def walk(oid, prefix=""):
        data = object_bytes(repo, oid)
        files = {}
        while data:
            header, data = data.split(b"\0", 1)
            mode, name = header.split(b" ", 1)
            child, data = data[:20].hex(), data[20:]
            path = prefix + name.decode()
            if mode == b"40000":
                files.update(walk(child, path + "/"))
            else:
                files[path] = object_bytes(repo, child)
        return files
    return walk(tree)

selected = replay("stage-selected", """click 337 116
wait 700
move 1220 800
shot staged-all
click 367 116
wait 700
move 1220 800
shot unstaged-all
click 476 267
wait 700
move 1220 800
shot staged
click 476 267
wait 700
move 1220 800
shot unstaged
click 476 267
wait 700
click 390 788
key cmd-a
type Commit selected file
click 307 116
move 1220 800
wait 1200
shot selected-committed
""")
files = committed_files(selected / "lib/.git")
assert files[f"notes/{doc_id}.xml"].decode() == body, "The + button must stage the selected body"
assert f"notes/{new['filename']}" not in files, "Unstaged Markdown must stay uncommitted"
assert "Files/attachment.bin" not in files, "Unstaged attachment must stay uncommitted"
assert len(json.loads(files["quiet.json"])["notes"]) == 1, "Unstaged metadata must stay uncommitted"
print(f"Git staging: +, unstage and selected-file commit verified; screenshots: {selected}")

discarded = replay("discard-tracked", """click 504 267
move 1220 800
wait 300
shot discard-confirmation
click 690 495
wait 200
click 504 267
wait 200
click 774 495
move 1220 800
wait 900
shot discarded
""")
assert (discarded / f"lib/notes/{doc_id}.xml").read_text() == seed["body"], "Discard must restore the indexed body"
assert (discarded / "lib/.git/refs/heads/main").read_text() == head_before, "Discard must not change HEAD"
assert (discarded / f"lib/notes/{new['filename']}").is_file(), "Discard must leave other files alone"

removed = replay("discard-new-note", """click 504 347
wait 300
click 774 495
move 1220 800
wait 900
shot removed-new-note
""")
assert not (removed / f"lib/notes/{new['filename']}").exists(), "Discard must delete an untracked note"
assert len(json.loads((removed / "lib/quiet.json").read_text())["notes"]) == 1, "Discard must remove the new manifest entry"
assert (removed / f"lib/notes/{doc_id}.xml").read_text() == body, "Discard must preserve other edits"
print(f"Git discard: confirmation, tracked restore and new-note removal verified; screenshots: {discarded}, {removed}")
