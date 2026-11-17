"""Manual update checks always answer, including repeated and unsupported checks.

Uses temporary libraries and a loopback server. No package is downloaded and
no installer runs. Build with --release --features devtools first.
"""
import http.server
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tempfile
import threading
import time

if sys.platform != 'darwin':
    sys.exit('This native UI harness currently runs on macOS.')
repo = Path(__file__).resolve().parents[2]
binary = repo / 'target/release/quiet_md'
if b'devtools: cannot read' not in binary.read_bytes():
    sys.exit('Build with cargo build --release --features devtools first.')
work = Path(tempfile.mkdtemp(prefix='quiet-update-check-'))
print(f'Manual update check output: {work}', flush=True)
version = re.search(r'^version = "([^"]+)"', (repo / 'Cargo.toml').read_text(), re.M)[1]
fixture = re.search(r'const TEST_SIG: &str = "(.*?)";', (repo / 'src/update.rs').read_text(), re.S)[1]
arch = 'arm64' if platform.machine() == 'arm64' else 'x86_64'
name = f'quiet-md-0.0.1-macos-{arch}.zip'
signature = fixture.replace('quiet-md-0.2.0-macos-arm64.zip', name)
requests = []
status = 200


class Server(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        assert self.path == f'/update-macos-{arch}.json', self.path
        requests.append(self.path)
        time.sleep(0.6)
        body = json.dumps({'version': '0.0.1', 'url': f'http://127.0.0.1:{self.server.server_port}/{name}',
                           'size': 8, 'signature': signature}).encode() if status == 200 else b''
        self.send_response(status)
        if status == 429:
            self.send_header('Retry-After', '120')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Server)
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    for case, status, expected in [('current', 200, 'current'), ('missing', 404, 'failed'),
                                    ('rate-limited', 429, 'failed'), ('unsupported', 200, 'unsupported')]:
        out = work / case
        library = out / 'library'
        (library / '.quiet').mkdir(parents=True)
        (library / 'Check.xml').write_text('<title>Check</title>\n<p>Keep this note.</p>\n')
        (library / '.quiet/format.json').write_text('{"format":4}')
        (library / '.quiet/index.json').write_text(json.dumps({'Check.xml': {
            'id': 'aaaaaaaa-0000-4000-8000-000000000001', 'created': 1, 'modified': 1}}))
        (library / '.quiet/preferences.json').write_text(json.dumps({
            'appearance': 'light', 'language': 'zh', 'english': False, 'seen_version': version,
            'auto_update': False, 'quick_hotkey': False, 'remote_images': False,
        }))
        script = ['resize 1200 900', 'move 1150 850', 'wait 3500', 'click 60 875',
                  'wait 300', 'click 120 254', 'wait 300', 'check-update-state idle silent']
        for attempt in range(2):
            script += ['click 1015 368']
            if case != 'unsupported' and not (case == 'rate-limited' and attempt == 1):
                script += ['wait 100', 'check-update-state checking silent']
            script += ['wait 1000', f'check-update-state {expected} feedback', f'shot result-{attempt}',
                       'move 1150 850', 'wait 3200', f'check-update-state {expected} silent']
        script += ['quit']
        steps = out / 'steps.txt'
        steps.write_text('\n'.join(script) + '\n')
        env = {**os.environ, 'QUIET_MD_HOME': str(library), 'QUIET_MD_LOG_DIR': str(out / 'logs'),
               'FDL_SCRIPT': str(steps), 'FDL_SHOTS': str(out), 'FDL_NOBLINK': '1'}
        for flag in ('QUIET_MD_UPDATE_URL', 'FDL_NOPARK', 'FDL_WINDOW_CAPTURE', 'QUIET_MD_ONBOARDING'):
            env.pop(flag, None)
        if case != 'unsupported':
            env['QUIET_MD_UPDATE_URL'] = f'http://127.0.0.1:{server.server_port}'
        requests.clear()
        with (out / 'log').open('w') as log:
            subprocess.run([str(binary)], cwd=out, env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=30)
        assert len(requests) == (0 if case == 'unsupported' else 1 if case == 'rate-limited' else 2), (case, requests)
        print(f'{case}: repeated checks showed feedback', flush=True)
finally:
    server.shutdown()
print(f'Manual update checks passed; screenshots: {work}')
