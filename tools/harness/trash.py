"""Browse deleted folders and document history using only a temporary library."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-trash-ui-"))
source = work / "source"
(source / ".quiet").mkdir(parents=True)
(source / "notes").mkdir()
folder = "11111111-1111-4111-8111-111111111111"
nested = "22222222-2222-4222-8222-222222222222"
xml = "33333333-3333-4333-8333-333333333333"
md = "44444444-4444-4444-8444-444444444444"
(source / ".quiet/format.json").write_text('{"format":9}')
(source / ".quiet/preferences.json").write_text(json.dumps({
    "appearance": "light", "english": True, "quick_hotkey": False,
}))

def deleted(id, title, parent=None, filename=None, root=None):
    return dict(id=id, title=title, folderId=parent, filename=filename, deletedAt=1794800000, rootId=root)

trash = [deleted(folder, "Deleted folder"), deleted(nested, "Nested", folder, root=folder),
         deleted(xml, "Deleted XML", nested, xml + ".xml", folder),
         deleted(md, "Deleted Markdown", nested, md + ".md", folder)]
(source / "quiet.json").write_text(json.dumps(dict(version=2, folders=[], notes=[], trash=trash)))
xml_body = "<title>Old stored title</title>\n<p>Current deleted document.</p>\n"
md_body = "# Markdown in Trash\n\nCurrent Markdown body.\n"
(source / "notes" / f"{xml}.xml").write_text(xml_body)
(source / "notes" / f"{md}.md").write_text(md_body)
old_xml = "<title>Earlier title</title>\n<p>Earlier XML body.</p>\n"
for id, ext, old in [(xml, "xml", old_xml), (md, "md", "# Earlier Markdown\n\nEarlier Markdown body.\n")]:
    history = source / ".quiet/history-trash" / id
    history.mkdir(parents=True)
    (history / f"1794700000000-aaaaaaaa.{ext}").write_text(old)
    (history / f"1794700000000-aaaaaaaa.{ext}.name").write_text("Before deletion")

script = work / "steps.txt"
script.write_text("""resize 1200 900
wait 800
click 217 875
wait 200
click 430 109
wait 200
click 430 109
wait 200
move 1150 850
shot nested-folder
click 430 109
wait 200
key e
type SHOULD_NOT_WRITE
wait 200
move 1150 850
shot readonly-document
click 1100 24
wait 300
move 1150 850
shot xml-history
type SHOULD_NOT_WRITE_HISTORY
click 950 877
wait 500
move 1150 850
shot history-copy
click 217 875
wait 200
click 430 109
wait 200
click 430 109
wait 200
rclick 430 165
wait 200
shot context-menu
click 490 220
wait 300
move 1150 850
shot markdown-history
key escape
key escape
wait 200
key e
type SHOULD_NOT_WRITE_MARKDOWN
click 1150 24
wait 250
click 735 485
wait 600
key cmd-down
type RESTORED_EDIT
wait 800
move 1150 850
shot restored-editable
""")
run = work / "run"
subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
               check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
lib = run / "lib"
manifest = json.loads((lib / "quiet.json").read_text())
assert {t["id"] for t in manifest["trash"]} == {folder, nested, xml}, manifest
assert (lib / "notes" / f"{xml}.xml").read_text() == xml_body, "Viewing Trash must not write the original"
copies = [n for n in manifest["notes"] if n["id"] != md]
assert len(copies) == 1 and "Before deletion" in copies[0]["title"], copies
copy_body = (lib / "notes" / copies[0]["filename"]).read_text()
assert "Earlier XML body." in copy_body and "Current deleted document." not in copy_body, copy_body
restored = (lib / "notes" / f"{md}.md").read_text().replace("\\_", "_")
assert "RESTORED_EDIT" in restored and "SHOULD_NOT_WRITE" not in restored, restored
for path in lib.rglob("*.xml"):
    assert "SHOULD_NOT_WRITE" not in path.read_text(), path
for path in (source / ".quiet/history-trash" / xml).iterdir():
    assert (lib / ".quiet/history-trash" / xml / path.name).read_bytes() == path.read_bytes()
assert (lib / ".quiet/history" / md / "1794700000000-aaaaaaaa.md").read_text().startswith("# Earlier Markdown")
assert "panicked" not in (run / "log").read_text()
print(f"Trash folders, readonly XML/Markdown, history copy and editing after restore verified; screenshots: {run}")
