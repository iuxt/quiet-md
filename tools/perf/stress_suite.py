#!/usr/bin/env python3
"""Synthetic UI/resource stress suite (macOS, release devtools, Pillow).

python3 tools/perf/stress_suite.py /tmp/quiet-stress [--cases idle,million]
Each case runs sequentially, with its own disposable Quiet.md library.
No system clipboard, focus activation, network or real notes are used.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import uuid
from xml.sax.saxutils import escape

HERE = Path(__file__).resolve().parent
PRELUDE = "resize 1280 800\nwait 1500\nmove 1230 750\n"


def open_note(title, settle=500):
    return f"key cmd-j\ntype {title}\nkey enter\nwait {settle}\n"


def edit_note(title, mixed=False):
    # The mixed fixture starts with a heading; edit its first ordinary paragraph.
    return (open_note(title) + "key cmd-up\nkey down\n" + ("key down\n" if mixed else "")
            + "key end\nwait 2000\n")


def library(root, docs):
    (root / ".quiet").mkdir(parents=True)
    (root / "Notes").mkdir()
    (root / ".quiet/preferences.json").write_text(json.dumps({
        "english": True, "appearance": "light", "theme": "craft", "quick_hotkey": False,
    }))
    notes = []
    for i, (title, body, ext) in enumerate(docs):
        key = str(uuid.uuid5(uuid.NAMESPACE_URL, f"quiet-stress-{i}-{title}"))
        text = f"<title>{escape(title)}</title>\n{body}\n" if ext == "xml" else f"# {title}\n\n{body}\n"
        (root / f"Notes/{key}.{ext}").write_text(text)
        notes.append({"id": key, "title": title, "filename": f"{key}.{ext}", "folderId": None,
                      "options": {"root_order": i, "created": 1794900000, "modified": 1794900000}})
    (root / "quiet.json").write_text(json.dumps({"version": 2, "folders": [], "notes": notes, "trash": []}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--cases", default="idle,million,undo,five-million,markdown,paragraph,code,table,library-1000,library-10000,search-hits,windows,photos,switching")
    args = parser.parse_args()
    root = args.output.resolve()
    root.mkdir(parents=True, exist_ok=True)
    if (root / "results.json").exists():
        parser.error("results.json already exists; choose a fresh output directory")
    seed = root / "million-seed"
    if not (seed / "quiet.json").is_file():
        subprocess.run([sys.executable, str(HERE / "gen_library.py"), str(seed)], check=True)
    million = max((seed / "Notes").glob("*.xml"), key=lambda p: p.stat().st_size).read_text()
    million = re.sub(r"^<title>.*?</title>\s*", "", million)
    small = [("Resource baseline", "<p>Resource stress test baseline.</p>", "xml")]
    cases = {}
    cases["idle"] = (small, PRELUDE + edit_note(small[0][0]) + "phase idle-before\nidle 60000\nphase sustained-edit\n" +
                     ("type x\nwait 100\n" * 120) + "wait 2000\nphase revert-before-save\n" +
                     ("type x\nkey backspace\nwait 800\n" * 8) + "phase save-clean\nbench 50 key cmd-s\nwait 2000\n"
                     "phase idle-after\nidle 60000\n", 210)
    edit = "phase typing\nbench 200 type x\nphase backspace\nbench 200 key backspace\nphase scrolling\nbench 200 wheel 900 500 40\n"
    cases["million"] = ([("Million", million, "xml")], PRELUDE + edit_note("Million", mixed=True) + edit +
                        "phase settle\nwait 3000\nphase idle-large\nidle 40000\n", 180)
    # Fill the history, then measure the 20 steps guaranteed by the undo budget.
    cases["undo"] = ([("Million", million, "xml"), ("Release", "<p>Small release control.</p>", "xml")], PRELUDE + edit_note("Million", mixed=True) +
                     "phase checkpoints\nbench 550 key enter\nphase undo\nbench 20 key cmd-z\n"
                     "phase redo\nbench 20 key cmd-shift-z\nphase leave\n" + open_note("Release", 3000) +
                     "phase idle-released\nidle 40000\n", 240)
    cases["five-million"] = ([("Five million", million * 5, "xml")], PRELUDE + edit_note("Five million", mixed=True) + edit +
                             "phase idle-large\nidle 40000\n", 240)
    md = "\n\n".join(f"Paragraph {i:05d}. " + "Markdown content 中文测试. " * 8 for i in range(6000))
    cases["markdown"] = ([("Large Markdown", md, "md")], PRELUDE + edit_note("Large Markdown") + edit +
                         "phase idle-large\nidle 15000\n", 180)
    cases["paragraph"] = ([("Long paragraph", "<p>" + "中文 mixed text 0123456789 " * 8000 + "</p>", "xml")],
                          PRELUDE + edit_note("Long paragraph") + edit + "phase settled\nidle 5000\n", 150)
    code = escape("\n".join(f'let value_{i} = "A long highlighted line with 中文, 0123456789";' for i in range(10000)))
    cases["code"] = ([("Long code", f'<pre lang="rust"><code>{code}</code></pre>', "xml")],
                     PRELUDE + edit_note("Long code") + "phase code-scroll\nbench 100 wheel 900 500 40\nphase code-edit\nbench 50 type x\nwait 2000\n", 180)
    table = '<table><colgroup>' + '<col width="100"/>' * 8 + '</colgroup><tbody>' + ''.join(
        '<tr>' + ''.join(f'<td><p>R{r} C{c} 中文</p></td>' for c in range(8)) + '</tr>' for r in range(1000)) + '</tbody></table>'
    cases["table"] = ([("Large table", table, "xml")], PRELUDE + edit_note("Large table") +
                      "phase table-scroll\nbench 100 wheel 900 500 40\nphase settled\nidle 5000\n", 180)
    for n in (1000, 10000):
        docs = [(f"Note {i:05d}", f"<p>Record {i:05d}. " + "Search body 中文 ABC def 0123456789. " * 60 + "</p>", "xml") for i in range(n)]
        steps = PRELUDE + "phase library-idle\nidle 15000\nphase home-scroll\nclick 70 65\nwait 500\nbench 100 wheel 900 500 40\n"
        steps += "phase sidebar-scroll\nbench 100 wheel 180 500 40\nphase search\nkey cmd-j\nbench 8 type q\n"
        steps += "key escape\nphase settle\nwait 3000\nphase library-idle-after\nidle 15000\n"
        steps += "phase edit-open\n" + edit_note("Note 00000")
        steps += "phase library-edit\n" + "type x\nwait 100\n" * 30 + "wait 2000\n"
        cases[f"library-{n}"] = (docs, steps, 240)
    many_docs = cases["library-10000"][0]
    cases["search-hits"] = (many_docs, PRELUDE + "phase search-open\nkey cmd-j\nwait 500\n"
                           "phase common-query\ntype Note\nwait 1500\nphase results-idle\nidle 10000\n"
                           "phase close-results\nkey escape\nwait 3000\nphase search-released\nidle 35000\n", 180)
    cases["windows"] = (many_docs, PRELUDE + "phase idle-before\nidle 15000\nphase churn-20\nchurn-windows 20\n"
                       "phase idle-after-20\nidle 20000\nphase churn-20-more\nchurn-windows 20\nphase idle-after-40\nidle 20000\n", 240)
    docs = [("Photo notes", "<p>Text only.</p>", "xml")]
    docs += [(f"Photo {i:02d}", f'<p>Photo test {i}</p><img src="assets/photos/{i}.jpg" width="600" height="450"/><p>After image.</p>', "xml") for i in range(24)]
    steps = PRELUDE + "phase photo-home\nclick 70 65\nwait 6000\n"
    for cycle in range(3):
        steps += f"phase photo-cycle-{cycle}\n"
        for i in range(cycle * 8, (cycle + 1) * 8):
            steps += open_note(f"Photo {i:02d}", 250)
        steps += open_note("Photo notes")
        steps += f"phase photo-release-{cycle}\nidle 35000\n"
    cases["photos"] = (docs, steps, 240)
    docs = [(f"Switch {i:03d}", "<p>" + "Switching note body 中文. " * 80 + "</p>", "xml") for i in range(160)]
    steps = PRELUDE
    for cycle in range(2):
        for chunk in range(4):
            steps += f"phase switch-{cycle}-{chunk}\n"
            for i in range(chunk * 40, (chunk + 1) * 40):
                steps += open_note(f"Switch {i:03d}", 20)
        steps += f"phase switch-idle-{cycle}\nidle 10000\n"
    cases["switching"] = (docs, steps, 300)
    results, failed = [], False
    for name in args.cases.split(","):
        docs, steps, deadline = cases[name]
        case = root / name
        source = case / "source"
        library(source, docs)
        if name == "photos":
            from PIL import Image
            photos = source / "assets/photos"
            photos.mkdir(parents=True)
            image = Image.open(HERE.parents[1] / "shared-mock/guides/assets/sample-photo.jpg").convert("RGB").resize((4032, 3024))
            for i in range(24):
                image.save(photos / f"{i}.jpg", quality=85)
        script = case / "script.txt"
        script.write_text(steps)
        print(f"\nCASE {name}: {len(docs)} documents", flush=True)
        run = subprocess.run([sys.executable, str(HERE / "resource_probe.py"), str(script), str(source), str(case / "run"),
                              "--timeout", str(deadline)])
        result = json.loads((case / "run/resources.json").read_text())
        result["case"] = name
        results.append(result)
        (root / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
        if run.returncode:
            failed = True
            print(f"CASE {name} failed or timed out; see {case / 'run/log'}", flush=True)
    print(f"Results: {root / 'results.json'}", flush=True)
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
