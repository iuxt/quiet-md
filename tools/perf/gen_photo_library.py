"""A library whose Home is full of phone-sized photos, for memory tests.

usage: python3 tools/perf/gen_photo_library.py <empty dir> [photos=8] [size=4032x3024]
Writes `photos` documents, each with a heading, a paragraph and one photo of
its own file (the same pixels, different paths: every one is decoded on its
own), plus a text-only document「Notes」opened last. Needs Pillow or macOS
`sips` to make the photo.
"""
import json, os, shutil, subprocess, sys, uuid

out = os.path.abspath(sys.argv[1])
n = int(sys.argv[2]) if len(sys.argv) > 2 else 8
w, h = (int(v) for v in (sys.argv[3] if len(sys.argv) > 3 else '4032x3024').split('x'))
here = os.path.dirname(os.path.abspath(__file__))
seed = os.path.join(here, '..', '..', 'shared-mock', 'guides', 'assets', 'sample-photo.jpg')
os.makedirs(os.path.join(out, 'Notes'), exist_ok=True)
os.makedirs(os.path.join(out, 'Images', 'photos'), exist_ok=True)
photo = os.path.join(out, 'Images', 'photos', '0.jpg')
try:
    from PIL import Image
    Image.open(seed).convert('RGB').resize((w, h)).save(photo, quality=85)
except ImportError:
    shutil.copy(seed, photo)
    subprocess.run(['sips', '-z', str(h), str(w), photo], check=True, stdout=subprocess.DEVNULL)
notes, t = [], 1794500000
for i in range(n):
    if i: shutil.copy(photo, os.path.join(out, 'Images', 'photos', f'{i}.jpg'))
    id = str(uuid.uuid5(uuid.NAMESPACE_URL, f'quiet-md-photo-{i}'))
    open(os.path.join(out, 'Notes', id + '.xml'), 'w').write(
        f'<title>Photo {i + 1}</title>\n<p>A trip, a dinner, a whiteboard: one phone photo in a short note.</p>\n'
        f'<img src="Images/photos/{i}.jpg" align="center"/>\n<p>Some words after the photo.</p>\n')
    notes.append({"id": id, "title": f"Photo {i + 1}", "filename": f"{id}.xml",
                  "createdAt": t + i, "updatedAt": t + i, "options": {"root_order": i}})
id = str(uuid.uuid5(uuid.NAMESPACE_URL, 'quiet-md-photo-notes'))
open(os.path.join(out, 'Notes', id + '.xml'), 'w').write('<title>Notes</title>\n<p>Only text here.</p>\n')
notes.append({"id": id, "title": "Notes", "filename": f"{id}.xml",
              "createdAt": t + n, "updatedAt": t + n, "options": {"root_order": n}})
os.makedirs(os.path.join(out, '.quiet'), exist_ok=True)
with open(os.path.join(out, 'quiet.json'), 'w') as f:
    json.dump({"version": 2, "folders": [], "notes": notes, "trash": []}, f, indent=2)
with open(os.path.join(out, '.quiet', 'preferences.json'), 'w') as f:
    f.write('{"english":true,"theme":"craft"}\n')
print(f'{n} photos of {w}x{h} in {out}')
