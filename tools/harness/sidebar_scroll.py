"""Visible sidebar rows must match the original pixels translated by scrolling."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

from PIL import Image, ImageChops

here = Path(__file__).resolve().parent
work = Path(tempfile.mkdtemp(prefix="quiet-sidebar-scroll-"))
for pinned in (False, True):
    case = work / ("pinned" if pinned else "plain")
    source = case / "source"
    (source / ".quiet").mkdir(parents=True)
    (source / "notes").mkdir()
    (source / ".quiet/format.json").write_text('{"format":6}')
    (source / ".quiet/preferences.json").write_text(json.dumps({
        "appearance": "light", "english": True, "quick_hotkey": False,
    }))
    notes = []
    for i in range(80):
        key = str(uuid.uuid4())
        title = f"Scroll note {i:02}" if i % 2 == 0 else f"滚动笔记 {i:02}"
        (source / f"notes/{key}.xml").write_text(f"<title>{title}</title>\n<p>Body</p>\n")
        notes.append({"id": key, "title": title, "filename": f"{key}.xml", "folderId": None,
                      "options": {"root_order": i, "pinned": pinned and i < 2}})
    (source / "quiet.json").write_text(json.dumps({"version": 2, "folders": [], "notes": notes, "trash": []}))
    # Small steps repeatedly cross row boundaries; large steps exercise stale bounds.
    deltas = [8] * 12 + [180, -180] + [-8] * 12
    steps = ["resize 1200 700", "wait 1600", "click 100 67", "move 1100 650", "wait 300", "shot initial"]
    for i, delta in enumerate(deltas):
        steps.extend([f"frame step-{i:02} wheel 180 400 {delta}"])
    for name, delta in (("bottom", 10000), ("bottom-again", 10000), ("top", -10000), ("top-again", -10000)):
        steps.extend([f"frame {name} wheel 180 400 {delta}", "noop", f"shot {name}-settled"])
    # Resize is delivered asynchronously; scroll the expanded viewport after it arrives.
    steps.extend(["wheel 180 400 1700", "wait 100", "resize 1200 500", "wait 100",
                  "resize 1200 900", "wait 100", "frame taller wheel 180 400 16", "noop", "shot taller-settled"])
    script = case / "steps.txt"
    script.write_text("\n".join(steps))
    run = case / "run"
    subprocess.run([sys.executable, str(here / "run.py"), str(script), str(source), str(run)],
                   check=True, timeout=60, env=dict(os.environ, FDL_NOBLINK="1"))
    with Image.open(run / "initial.png") as image:
        reference = image.convert("RGB")
    scale = reference.width / 1200
    offset = 0
    for i, delta in enumerate(deltas):
        offset += delta
        # Stay inside the scroll viewport, excluding icons, hover and fixed nav.
        box = tuple(round(v * scale) for v in (40, 100, 210, 300))
        shifted = (box[0], box[1] + round(offset * scale), box[2], box[3] + round(offset * scale))
        with Image.open(run / f"step-{i:02}.png") as image:
            actual = image.convert("RGB").crop(box)
        diff = ImageChops.difference(reference.crop(shifted), actual)
        assert diff.getbbox() is None, f"{case.name}, step {i}, scroll {offset}: visible row pixels disappeared or changed"
    for name in ("bottom", "bottom-again", "top", "top-again", "taller"):
        box = tuple(round(v * scale) for v in (40, 100, 210, 640))
        with Image.open(run / f"{name}.png") as image:
            actual = image.convert("RGB").crop(box)
        with Image.open(run / f"{name}-settled.png") as image:
            expected = image.convert("RGB").crop(box)
        assert expected.convert("L").getextrema()[0] < 100, f"{case.name}, {name}: missing reference text"
        assert ImageChops.difference(expected, actual).getbbox() is None, f"{case.name}, {name}: boundary frame lost visible text"
print(f"Sidebar scrolling: first-frame text, fast jumps, reverse scrolling, pinned rows, boundaries and resizing verified; screenshots: {work}")
