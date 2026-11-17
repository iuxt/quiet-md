#!/usr/bin/env python3
"""Keep a certificate-backed identity across local rebuilds, without changing trust settings."""
import fcntl
import hashlib
import os
from pathlib import Path
import secrets
import ssl
import subprocess
import sys
import tempfile


def run(*args, env=None):
    result = subprocess.run(args, capture_output=True, text=True, env=env)
    if result.returncode:
        raise RuntimeError(f"{args[0]}: {result.stderr.strip()}")
    return result.stdout


def sign(app, state):
    os.umask(0o077)
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    with (state / "lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        keychain = state / "identity.keychain-db"
        password_file = state / "password"
        certificate = state / "certificate.pem"
        files = (keychain, password_file, certificate)
        if any(p.exists() for p in files) and not all(p.is_file() for p in files):
            raise RuntimeError(f"Incomplete signing identity in {state}; restore its files before rebuilding. "
                               "Creating a new identity would require granting permissions again.")
        if not keychain.exists():
            print(f"Creating persistent local signing identity in {state}", flush=True)
            password = secrets.token_hex(32)
            with tempfile.TemporaryDirectory(prefix="quiet-md-signing-") as tmp:
                tmp = Path(tmp)
                cert, key, p12 = tmp / "certificate.pem", tmp / "key.pem", tmp / "identity.p12"
                run("/usr/bin/openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                    "-keyout", str(key), "-out", str(cert), "-days", "36500",
                    "-subj", "/CN=Quiet.md Local Development",
                    "-addext", "keyUsage=critical,digitalSignature",
                    "-addext", "extendedKeyUsage=critical,codeSigning",
                    "-addext", "basicConstraints=critical,CA:FALSE")
                run("/usr/bin/openssl", "pkcs12", "-export", "-inkey", str(key), "-in", str(cert),
                    "-out", str(p12), "-passout", "env:QUIET_MD_SIGNING_PASSWORD",
                    env={**os.environ, "QUIET_MD_SIGNING_PASSWORD": password})
                # Never replace an existing identity after an initialization error.
                try:
                    password_file.write_text(password)
                    certificate.write_bytes(cert.read_bytes())
                    run("security", "create-keychain", "-p", password, str(keychain))
                    run("security", "unlock-keychain", "-p", password, str(keychain))
                    run("security", "import", str(p12), "-k", str(keychain), "-P", password,
                        "-T", "/usr/bin/codesign")
                    run("security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s",
                        "-k", password, str(keychain))
                except RuntimeError:
                    if keychain.exists():
                        run("security", "delete-keychain", str(keychain))
                    password_file.unlink(missing_ok=True)
                    certificate.unlink(missing_ok=True)
                    raise
        password = password_file.read_text().strip()
        run("security", "unlock-keychain", "-p", password, str(keychain))
        try:
            der = ssl.PEM_cert_to_DER_cert(certificate.read_text())
            identity = hashlib.sha1(der).hexdigest()
            run("codesign", "--force", "--timestamp=none", "--keychain", str(keychain),
                "--sign", identity, str(app))
            run("codesign", "--verify", "--deep", "--strict", str(app))
        finally:
            run("security", "lock-keychain", str(keychain))
        print(f"Local signing identity: {identity}")


if __name__ == "__main__":
    if sys.platform != "darwin" or len(sys.argv) != 2:
        sys.exit("Usage (macOS only): sign-local-macos.py <app>")
    state = Path(os.environ.get("QUIET_MD_SIGNING_DIR",
                 str(Path.home() / "Library/Application Support/quiet-md/local-signing"))).expanduser().resolve()
    try:
        sign(Path(sys.argv[1]).resolve(), state)
    except (OSError, RuntimeError, ValueError) as error:
        sys.exit(f"local signing: {error}")
