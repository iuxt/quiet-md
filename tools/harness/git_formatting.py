"""Formatted XML Git comparisons in a throwaway library.

Build with --release --features devtools. Covers style-only changes, nested
blocks, tables, code, equations, source switching and read-only protection.
"""
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile

here = Path(__file__).resolve().parent
seed = runpy.run_path(str(here / "git_sync.py"))
work = Path(tempfile.mkdtemp(prefix="quiet-git-formatting-"))
source = work / "source"
shutil.copytree(seed["lib"], source)
note = Path(f"notes/{seed['doc_id']}.xml")
body = '''<title>Git regression</title>
<p><b>Keep this body.</b></p>
<h2>Formatted comparison</h2>
<p><b>Bold</b> · <em>Italic</em> · <u>Underline</u> · <del>Deleted</del> · <code>inline code</code></p>
<ul><li>List item<ul><li>Nested item</li></ul></li></ul>
<ol><li>First</li><li>Second</li></ol>
<checkbox done="true">Read-only task</checkbox>
<blockquote><p>Quoted <em>text</em></p></blockquote>
<callout emoji="💡" background-color="light-orange"><p>Callout</p></callout>
<table><colgroup><col width="240"/><col width="240"/></colgroup><tr><td><p><b>Table</b></p></td><td><p>Cell <latex>\\frac{1}{2}</latex></p></td></tr></table>
<pre lang="rust"><code>fn main() { println!("Formatted diff preserves code and supports long lines in a narrow window."); }</code></pre>
<p>Formula <latex>E=mc^2</latex></p>
'''
(source / note).write_text(body)
script = work / "steps.txt"
script.write_text("""resize 1280 860
wait 1400
move 1220 800
click 110 98
wait 1000
shot formatted-top
click 608 594
type SHOULD_NOT_WRITE
wheel 1000 600 350
wait 400
shot formatted-bottom
click 1205 150
move 1220 800
wait 400
shot source
click 1205 150
move 1220 800
wait 400
shot formatted-returned
resize 780 640
wait 400
shot narrow
wheel 700 500 100000
wait 400
shot narrow-bottom
""")
run = work / "run"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
assert (run / "lib" / note).read_text() == body, "Formatted comparison must stay read-only"
for path in (".git/index", ".git/refs/heads/main"):
    assert (run / "lib" / path).read_bytes() == (source / path).read_bytes(), path
for name in ("formatted-top", "formatted-bottom", "source", "formatted-returned", "narrow", "narrow-bottom"):
    assert (run / f"{name}.png").is_file(), name
assert (run / "formatted-top.png").read_bytes() != (run / "formatted-bottom.png").read_bytes(), "Scroll must move content"
assert "panicked" not in (run / "log").read_text()
print(f"Formatted Git diff, source switching, resizing and read-only checks passed; screenshots: {run}")

style_body = seed["body"].replace("Keep this body.", "<b>Keep this body.</b>")
(source / note).write_text(style_body)
script.write_text("resize 1280 860\nwait 1400\nmove 1220 800\nclick 110 98\nwait 1000\nshot style-only\ntype SHOULD_NOT_WRITE\n")
style_run = work / "style-only"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(style_run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
assert (style_run / "lib" / note).read_text() == style_body
assert (style_run / "style-only.png").is_file()
assert "panicked" not in (style_run / "log").read_text()
print(f"Style-only comparison verified; screenshot: {style_run / 'style-only.png'}")
