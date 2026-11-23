"""Search ranking, virtual rows, keyboard reveal, empty results and save target.
SEARCH_BASELINE enables exact screenshots and saved-body comparison.
"""
import json,os,subprocess,sys,tempfile,uuid
from pathlib import Path
here=Path(__file__).resolve().parent
root=Path(tempfile.mkdtemp(prefix='quiet-search-results-'))
source=root/'source';(source/'.quiet').mkdir(parents=True);(source/'Notes').mkdir()
(source/'.quiet/format.json').write_text('{"format":11}')
(source/'.quiet/preferences.json').write_text(json.dumps({'english':True,'appearance':'light','theme':'craft','quick_hotkey':False}))
notes=[]
for i in range(100):
 id=str(uuid.uuid5(uuid.NAMESPACE_URL,f'quiet-search-row-{i}'))
 title=f'{"Needle" if i%2==0 else "Document"} {i:03d}'
 (source/f'Notes/{id}.xml').write_text(f'<title>{title}</title>\n<p>Needle body {i:03d}. '+('Search context 中文. '*(1+i%5))+'</p>\n')
 notes.append({'id':id,'filename':f'{id}.xml','title':title,'folderId':None,'options':{'root_order':i}})
(source/'quiet.json').write_text(json.dumps({'version':2,'folders':[],'notes':notes,'trash':[]}))
script=root/'script.txt';script.write_text('''resize 1280 800
wait 1400
move 1230 750
key cmd-j
wait 400
shot recent
key cmd-a
type Needle
wait 400
shot first
bench 99 key down
wait 300
shot last
key cmd-a
type NoSuchMatch
wait 300
shot empty
key cmd-a
key backspace
wait 300
shot cleared
type Needle
bench 99 key down
key enter
wait 500
type MARKER
key cmd-s
wait 800
shot opened
''')
baseline=os.environ.get('SEARCH_BASELINE')
if baseline:
 binary=os.environ.get('FDL_BIN') or str(here.parents[1]/'target/release/quiet_md')
 subprocess.run([sys.executable,str(here/'compare.py'),str(script),baseline,binary,str(root/'compare'),str(source)],check=True,timeout=180)
 run=root/'compare/b'
else:
 run=root/'run'
 subprocess.run([sys.executable,str(here/'run.py'),str(script),str(source),str(run)],check=True,timeout=90)
assert 'panicked' not in (run/'log').read_text()
changed=[p for p in (run/'lib/Notes').glob('*.xml') if 'MARKER' in p.read_text()]
last=max(note['id'] for i,note in enumerate(notes) if i%2)
assert len(changed)==1 and changed[0].stem==last, 'Keyboard selection must reveal and open the final body-ranked hit (stable UUID order)'
if baseline:
 for p in (run/'lib/Notes').glob('*.xml'):
  assert p.read_bytes()==(root/'compare/a/lib/Notes'/p.name).read_bytes(),p
print(f'Search ranking, keyboard reveal, empty results and saved target verified: {root}',flush=True)
