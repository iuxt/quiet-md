"""Large tables and code: scrolling, spanning cells, find, editing and undo.
LARGE_CONTENT_BASELINE enables exact screenshot comparisons with an older devtools binary.
Only synthetic temporary libraries and the harness clipboard are used.
"""
import json,os,subprocess,sys,uuid,tempfile
from pathlib import Path
from xml.sax.saxutils import escape
here=Path(__file__).resolve().parent
root=Path(tempfile.mkdtemp(prefix='quiet-large-content-'))
source=root/'source';(source/'.quiet').mkdir(parents=True,exist_ok=True);(source/'notes').mkdir(exist_ok=True)
id=str(uuid.uuid5(uuid.NAMESPACE_URL,'quiet-large-content-regression'))
(source/'.quiet/format.json').write_text('{"format":9}')
(source/'.quiet/preferences.json').write_text(json.dumps({'english':True,'appearance':'light','theme':'craft','quick_hotkey':False}))
rows=[]
for r in range(220):
 cells=[]
 for c in range(4):
  if c==0 and r in (6,7):continue
  span=' rowspan="3"' if c==0 and r==5 else ''
  content=f'R{r:03d} C{c}. '+('Wrapped cell 中文 text. '*5 if r%11==0 else 'Body.')
  cells.append(f'<td{span}><p>{content}</p></td>')
 rows.append('<tr>'+''.join(cells)+'</tr>')
code='\n'.join(f'let value_{i:03d} = "中文 code line {i}";' for i in range(300))
body='<title>Large rendering</title>\n<table><colgroup>'+''.join('<col width="150"/>' for _ in range(4))+'</colgroup><thead><tr>'+''.join(f'<th><p>Column {c}</p></th>' for c in range(4))+'</tr></thead><tbody>'+''.join(rows)+'</tbody></table>\n<pre lang="Rust"><code>'+escape(code)+'</code></pre>\n<p>After code.</p>\n'
(source/f'notes/{id}.xml').write_text(body)
(source/'quiet.json').write_text(json.dumps({'version':2,'folders':[],'notes':[{'id':id,'filename':f'{id}.xml','title':'Large rendering','folderId':None,'options':{'root_order':0}}],'trash':[]}))
script=root/'script.txt';script.write_text('''resize 1280 800
wait 1400
move 1230 750
shot table-top
wheel 900 500 1200
wait 300
shot table-middle
frame table-reverse wheel 900 500 -400
key cmd-f
type R190 C2
wait 500
key escape
wait 400
shot table-found
type MARKER
wait 1500
shot table-edited
check-render-cache
key cmd-z
wait 800
shot table-undo
key cmd-shift-z
wait 800
shot table-redo
check-render-cache
key cmd-f
key cmd-a
type value_250
wait 400
key escape
wait 300
shot code-middle
key cmd-down
wait 300
shot code-end
resize 980 650
wait 600
shot code-narrow
''')
print(source,script)

baseline=os.environ.get('LARGE_CONTENT_BASELINE')
for theme in os.environ.get('LARGE_CONTENT_THEMES','craft,feishu').split(','):
 (source/'.quiet/preferences.json').write_text(json.dumps({'english':True,'appearance':'light','theme':theme,'quick_hotkey':False}))
 output=root/theme
 if baseline:
  binary=os.environ.get('FDL_BIN') or str(here.parents[1]/'target/release/quiet_md')
  subprocess.run([sys.executable,str(here/'compare.py'),str(script),baseline,binary,str(output),str(source)],check=True,timeout=180)
  run=output/'b'
 else:
  subprocess.run([sys.executable,str(here/'run.py'),str(script),str(source),str(output)],check=True,timeout=100)
  run=output
 log=(run/'log').read_text()
 assert 'panicked' not in log and 'PANIC' not in log,log
 actual=(run/f'lib/notes/{id}.xml').read_text()
 import xml.etree.ElementTree as ET
 doc=ET.fromstring('<doc>'+actual+'</doc>')
 marked=[row for row in doc.iter('tr') if 'MARKER' in ''.join(row.itertext())]
 assert len(marked)==1 and 'R190 C0' in ''.join(marked[0][0].itertext()) and ''.join(marked[0][2].itertext())=='MARKER. Body.', 'Find must reveal and replace the offscreen target match; undo/redo must preserve it'
 assert len(list(run.glob('*.png')))==10, 'Missing frames'
 if baseline:assert actual==(output/f'a/lib/notes/{id}.xml').read_text(),'Optimized editing changed document structure'
 print(f'{theme}: large table heights, rowspan, first-frame scrolling, find/reveal, undo/redo and long code screenshots verified: {run}',flush=True)
