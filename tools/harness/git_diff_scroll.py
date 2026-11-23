"""Long Git diffs: wrapped rows, scrolling, mode changes and resizing.

Build with --release --features devtools. GIT_DIFF_BASELINE optionally names
an older devtools binary for pixel comparisons and before/after timings.
Only temporary libraries are used; no network, system Git or clipboard.
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
work = Path(tempfile.mkdtemp(prefix="quiet-git-diff-scroll-"))
source = work / "source"
shutil.copytree(seed["lib"], source)
note = Path(f"Notes/{seed['doc_id']}.xml")
body = "<title>Git regression</title>\n" + "\n".join(
    f"<p>Line {i:04d}: " + ("Long wrapped paragraph with 中文内容 and punctuation. " * 12 if i % 7 == 0 else "Changed text.") + "</p>"
    for i in range(1250)
) + "\n"
(source / note).write_text(body)
metadata = json.loads((source / "quiet.json").read_text())
md = dict(metadata["notes"][0])
md.update(id="bbbbbbbb-0000-4000-8000-000000000002", filename="bbbbbbbb-0000-4000-8000-000000000002.md", title="Long Markdown")
metadata["notes"].append(md)
(source / "quiet.json").write_text(json.dumps(metadata))
markdown = Path("Notes") / md["filename"]
md_body = "# Long Markdown\n\n" + "\n".join(
    f"Line {i:04d}: " + ("Long wrapped paragraph with 中文内容 and punctuation. " * 12 if i % 7 == 0 else "Changed text.")
    for i in range(1250)
) + "\n"
(source / markdown).write_text(md_body)
script = work / "scroll.txt"
script.write_text("""resize 1280 860
wait 1400
move 1220 800
click 110 98
wait 1200
click 380 267
wait 600
move 1000 600
shot text-top
bench 80 wheel 1000 600 32
shot text-scrolled
bench 80 wheel 1000 600 -32
wheel 1000 600 1000000
wait 300
shot text-bottom
click 380 187
wait 600
click 1205 150
move 1000 600
wait 300
wheel 1000 600 -1000000
wait 300
shot source-top
bench 80 wheel 1000 600 32
shot source-scrolled
wheel 1000 600 -1000000
wait 300
resize 780 640
move 740 600
wait 400
shot narrow-source
bench 80 wheel 700 500 32
wheel 700 500 -1000000
wait 300
shot narrow-top
resize 1280 860
move 1220 800
wait 400
shot wide-again
""")
baseline = os.environ.get("GIT_DIFF_BASELINE")
if baseline:
    binary = os.environ.get("FDL_BIN") or str(here.parents[1] / "target/release/quiet_md")
    subprocess.run([sys.executable, str(here / "compare.py"), str(script), baseline, binary,
                    str(work / "compare"), str(source)], check=True, timeout=180)
    runs = [work / "compare/a", work / "compare/b"]
else:
    target = work / "run"
    subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(target)],
                   check=True, timeout=90, env=dict(os.environ, FDL_NOBLINK="1"))
    runs = [target]
for run in runs:
    assert (run / "lib" / note).read_text() == body, "Diff must remain read-only"
    assert (run / "lib" / markdown).read_text() == md_body, "Markdown diff must remain read-only"
    for mode in ("text", "source"):
        assert (run / f"{mode}-top.png").read_bytes() != (run / f"{mode}-scrolled.png").read_bytes(), "Scroll must move the content"
    for path in (".git/index", ".git/refs/heads/main"):
        assert (run / "lib" / path).read_bytes() == (source / path).read_bytes(), path
    log = (run / "log").read_text()
    assert "panicked" not in log, log
    print(f"Scroll timings ({run}):")
    print("\n".join(line for line in log.splitlines() if line.startswith("bench ")))
print(f"Long Git diff scroll, wrapping, resizing and read-only checks passed: {work}")
