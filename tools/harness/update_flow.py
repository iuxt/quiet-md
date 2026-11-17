"""Exercise the actual Update button and reject a tampered download.

Build with --release --features devtools. Uses a loopback server, a temporary
library and a deliberately invalid signature; no installer can run.
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
work = Path(tempfile.mkdtemp(prefix='quiet-update-ui-'))
print(f'Update UI output: {work}', flush=True)
library = work / 'library'
(library / '.quiet').mkdir(parents=True)
version = re.search(r'^version = "([^"]+)"', (repo / 'Cargo.toml').read_text(), re.M)[1]
(library / 'Update regression.xml').write_text('<title>Update regression</title>\n<p>Keep this note.</p>\n')
(library / '.quiet/format.json').write_text('{"format":4}')
(library / '.quiet/index.json').write_text(json.dumps({'Update regression.xml': {'id': 'aaaaaaaa-0000-4000-8000-000000000001', 'created': 1, 'modified': 1}}))
(library / '.quiet/preferences.json').write_text(json.dumps({
    'appearance': 'light', 'language': 'zh', 'english': False, 'seen_version': version,
    'auto_update': True, 'quick_hotkey': False, 'remote_images': False,
}))
# Rewriting the trusted comment leaves a decodable signature, but breaks its
# authentication. This package must never reach Ready or the installer.
fixture = re.search(r'const TEST_SIG: &str = "(.*?)";', (repo / 'src/update.rs').read_text(), re.S)[1]
arch = 'arm64' if platform.machine() == 'arm64' else 'x86_64'
name = f'quiet-md-99.0.0-macos-{arch}.zip'
signature = fixture.replace('quiet-md-0.2.0-macos-arm64.zip', name)
payload = b'tampered update' * 150000
requests = []


class Server(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        requests.append(self.path)
        if self.path == f'/update-macos-{arch}.json':
            body = json.dumps({'version': '99.0.0', 'url': f'http://127.0.0.1:{self.server.server_port}/{name}',
                               'size': len(payload), 'signature': signature}).encode()
            self.send_response(200)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == f'/{name}':
            self.send_response(200)
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            try:
                for start in range(0, len(payload), 32768):
                    self.wfile.write(payload[start:start + 32768])
                    self.wfile.flush()
                    time.sleep(0.05)
            except (BrokenPipeError, ConnectionResetError):
                pass
        else:
            self.send_error(404)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Server)
threading.Thread(target=server.serve_forever, daemon=True).start()
steps = work / 'steps.txt'
steps.write_text('''resize 1200 900
move 1150 850
wait 12000
check-update-state available
shot update-available
key cmd-,
wait 300
click 120 260
wait 300
shot update-settings
key escape
wait 300
click 244 24
wait 500
check-update-state downloading
shot update-progress
wait 5000
check-update-state failed
key cmd-,
wait 300
click 120 260
wait 300
shot update-rejected
check-save-safety setup
wait 300
check-save-safety finish
wait 300
quit
''')
env = {**os.environ, 'QUIET_MD_HOME': str(library), 'QUIET_MD_LOG_DIR': str(work / 'logs'),
       'QUIET_MD_UPDATE_URL': f'http://127.0.0.1:{server.server_port}',
       'FDL_SCRIPT': str(steps), 'FDL_SHOTS': str(work), 'FDL_NOBLINK': '1'}
# Never inherit a user's request to bring scripted windows to the foreground.
for flag in ('FDL_NOPARK', 'FDL_WINDOW_CAPTURE', 'QUIET_MD_ONBOARDING'):
    env.pop(flag, None)
try:
    with (work / 'log').open('w') as log:
        subprocess.run([str(binary)], cwd=work, env=env, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=55)
finally:
    server.shutdown()
log = (work / 'log').read_text()
for state in ('available', 'downloading', 'failed'):
    assert f'update-state: {state}' in log, (state, work)
assert 'update-state: ready' not in log and 'update-state: installing' not in log
assert requests.count(f'/{name}') == 1, (requests, work)
assert f'/update-macos-{arch}.json' in requests
assert 'save-safety: PASS' in log, work
for shot in ('update-available', 'update-settings', 'update-progress', 'update-rejected'):
    assert (work / f'{shot}.png').is_file(), (shot, work)
print(f'Update UI: button, progress, signature rejection and multi-window save safety passed; screenshots: {work}')
