"""Exercise embedded SSH Git and HTTPS LFS against disposable local servers.

Requires Python paramiko (install in a temporary venv). System Git serves only
this test's bare repository; the application still uses embedded Git and LFS.
"""
import datetime
import json
import os
from pathlib import Path
import shlex
import socket
import ssl
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import paramiko
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

repo = Path(__file__).resolve().parents[2]
with tempfile.TemporaryDirectory(prefix="quiet-ssh-lfs-") as directory:
    work = Path(directory)
    host_key = paramiko.RSAKey.generate(2048)
    client_key = paramiko.RSAKey.generate(2048)
    client_key.write_private_key_file(str(work / "identity"), password="test-key-password")
    ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    server_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    now = datetime.datetime.now(datetime.timezone.utc)
    ca_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Quiet test CA")])
    ca = (x509.CertificateBuilder().subject_name(ca_name).issuer_name(ca_name).public_key(ca_key.public_key())
          .serial_number(x509.random_serial_number()).not_valid_before(now - datetime.timedelta(days=1))
          .not_valid_after(now + datetime.timedelta(days=1)).add_extension(x509.BasicConstraints(ca=True, path_length=None), True)
          .sign(ca_key, hashes.SHA256()))
    leaf = (x509.CertificateBuilder().subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")]))
            .issuer_name(ca_name).public_key(server_key.public_key()).serial_number(x509.random_serial_number())
            .not_valid_before(now - datetime.timedelta(days=1)).not_valid_after(now + datetime.timedelta(days=1))
            .add_extension(x509.SubjectAlternativeName([x509.DNSName("localhost")]), False).sign(ca_key, hashes.SHA256()))
    (work / "ca.pem").write_bytes(ca.public_bytes(serialization.Encoding.PEM))
    (work / "server.pem").write_bytes(leaf.public_bytes(serialization.Encoding.PEM))
    (work / "server.key").write_bytes(server_key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    objects = {}
    operations = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def reply(self, data):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            if self.path.endswith("/objects/batch"):
                assert self.headers.get("Authorization") == "RemoteAuth fixture-token"
                upload = body["operation"] == "upload"
                operations.append(body["operation"])
                result = []
                for pointer in body["objects"]:
                    oid = pointer["oid"]
                    actions = {} if upload and oid in objects else {
                        "upload" if upload else "download": {"href": f"{endpoint}/objects/{oid}"}}
                    if upload and actions:
                        actions["verify"] = {"href": f"{endpoint}/verify"}
                    result.append(dict(pointer, actions=actions))
                self.reply(json.dumps({"transfer": "basic", "objects": result}).encode())
            else:
                assert self.path.endswith("/verify") and body["oid"] in objects
                assert not self.headers.get("Authorization")
                operations.append("verify")
                self.reply(b"{}")

        def do_PUT(self):
            assert not self.headers.get("Authorization")
            objects[self.path.rsplit("/", 1)[1]] = self.rfile.read(int(self.headers["Content-Length"]))
            self.reply(b"")

        def do_GET(self):
            assert not self.headers.get("Authorization")
            self.reply(objects[self.path.rsplit("/", 1)[1]])

    http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(work / "server.pem", work / "server.key")
    http.socket = context.wrap_socket(http.socket, server_side=True)
    endpoint = f"https://localhost:{http.server_port}/notes.git/info/lfs"
    threading.Thread(target=http.serve_forever, daemon=True).start()
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    ssh_port = listener.getsockname()[1]
    (work / "known_hosts").write_text(f"[localhost]:{ssh_port} {host_key.get_name()} {host_key.get_base64()}\n")
    (work / ".gitconfig").write_text(f'[http "https://localhost:{http.server_port}"]\n sslCAInfo = {work / "ca.pem"}\n')

    class Server(paramiko.ServerInterface):
        def __init__(self):
            self.command = None
            self.ready = threading.Event()

        def get_allowed_auths(self, _):
            return "publickey"

        def check_auth_publickey(self, username, key):
            return paramiko.AUTH_SUCCESSFUL if username == "git" and key == client_key else paramiko.AUTH_FAILED

        def check_channel_request(self, kind, _):
            return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

        def check_channel_exec_request(self, channel, command):
            self.command = shlex.split(command.decode())
            self.ready.set()
            return True

    def connection(sock):
        transport = paramiko.Transport(sock)
        transport.add_server_key(host_key)
        server = Server()
        try:
            transport.start_server(server=server)
            channel = transport.accept(20)
            if channel is None or not server.ready.wait(20):
                return
            command = server.command
            assert command[1].lstrip("/") == "notes.git", command
            if command[0] == "git-lfs-authenticate":
                assert command[2] in ("upload", "download")
                channel.sendall(json.dumps({"href": endpoint, "header": {"Authorization": "RemoteAuth fixture-token"}}).encode())
                channel.send_exit_status(0)
                channel.shutdown_write()
            else:
                assert command[0] in ("git-upload-pack", "git-receive-pack"), command
                process = subprocess.Popen([command[0], str(work / "bare.git")], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)

                def feed():
                    try:
                        while data := channel.recv(65536):
                            process.stdin.write(data)
                            process.stdin.flush()
                    except (OSError, EOFError):
                        pass
                    finally:
                        try:
                            process.stdin.close()
                        except BrokenPipeError:
                            pass

                threading.Thread(target=feed, daemon=True).start()
                while data := os.read(process.stdout.fileno(), 65536):
                    channel.sendall(data)
                channel.send_exit_status(process.wait(timeout=20))
                channel.shutdown_write()
            deadline = time.monotonic() + 5
            while transport.is_active() and not channel.closed and time.monotonic() < deadline:
                time.sleep(0.01)
            channel.close()
        except (EOFError, ConnectionResetError, paramiko.SSHException):
            pass  # Expected when a negative authentication test disconnects.
        finally:
            transport.close()

    def accept():
        while True:
            try:
                sock, _ = listener.accept()
            except OSError:
                return
            threading.Thread(target=connection, args=(sock,), daemon=True).start()

    threading.Thread(target=accept, daemon=True).start()
    env = dict(os.environ, QUIET_MD_TEST_REMOTE=f"ssh://git@localhost:{ssh_port}/notes.git", QUIET_MD_TEST_CA=str(work / "ca.pem"),
               QUIET_MD_TEST_KEY=str(work / "identity"), QUIET_MD_TEST_BARE=str(work / "bare.git"), QUIET_MD_TEST_TLS=endpoint,
               QUIET_MD_GIT_KNOWN_HOSTS=str(work / "known_hosts"), QUIET_MD_TEST_CONFIG_DIR=str(work))
    try:
        subprocess.run(["cargo", "test", "--release", "--locked", "-p", "fdl_core", "ssh_push_clone_pull_with_self_signed_lfs", "--", "--ignored", "--nocapture"],
                       cwd=repo, env=env, check=True, timeout=180)
        assert "upload" in operations and "download" in operations and "verify" in operations, operations
        print("SSH custom port, encrypted key, host verification, clone/push/pull and self-signed LFS upload/verify/download passed")
    finally:
        listener.close()
        http.shutdown()
