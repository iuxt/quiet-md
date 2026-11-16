#!/usr/bin/env python3
"""Prepare signed update assets, or publish a verified draft to iuxt/quiet-md.

Dry-run is read-only. --prepare writes to a new directory and signs with minisign.
--publish additionally uploads, verifies a draft, and makes it the latest release.
Private keys are never copied into the output. gh supplies publishing credentials.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
REPO = "iuxt/quiet-md"
BASE = f"https://github.com/{REPO}/releases"
UPSTREAM_KEY = "RWTt2S54fS5BxS1ryw79mP2a0osMiOjhrYyDrun+YNU2W6nq+HTqG7Jm"
PLATFORMS = {
    "macos-arm64": "quiet-md-{version}-macos-arm64.zip",
    "macos-x86_64": "quiet-md-{version}-macos-x86_64.zip",
    "windows-x64": "quiet-md-{version}-windows-x64-setup.exe",
    "linux-x86_64": "quiet-md-{version}-x86_64.AppImage",
}


def run(*args, capture=False):
    result = subprocess.run([str(a) for a in args], check=True, cwd=ROOT,
                            stdout=subprocess.PIPE if capture else None, text=capture)
    return result.stdout.strip() if capture else None


def version():
    versions = []
    for name in ("Cargo.toml", "crates/core/Cargo.toml"):
        match = re.search(r'^version = "([^"]+)"$', (ROOT / name).read_text(), re.M)
        if not match or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", match[1]):
            raise ValueError(f"Invalid release version: {name}")
        versions.append(match[1])
    if versions[0] != versions[1]:
        raise ValueError("Both Cargo.toml versions must match")
    return versions[0]


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def inventory(packages, release):
    expected = {pattern.format(version=release): platform for platform, pattern in PLATFORMS.items()}
    found = {}
    for path in packages:
        path = path.resolve(strict=True)
        platform = expected.get(path.name)
        if platform is None or platform in found:
            raise ValueError(f"Unexpected or duplicate package: {path.name}")
        size = path.stat().st_size
        if not path.is_file() or not 0 < size <= 1 << 30:
            raise ValueError(f"Invalid package size: {path}")
        checksum = digest(path)
        sidecar = Path(str(path) + ".sha256")
        if sidecar.exists() and (not sidecar.read_text().split() or sidecar.read_text().split()[0].lower() != checksum):
            raise ValueError(f"Checksum mismatch: {path}")
        found[platform] = {"path": path, "version": release, "size": size, "sha256": checksum,
                           "url": f"{BASE}/download/v{release}/{path.name}"}
    if set(found) != set(PLATFORMS):
        raise ValueError(f"All four platforms are required; missing: {', '.join(sorted(set(PLATFORMS) - set(found)))}")
    return found


def public_key():
    path = ROOT / "packaging/update-key.pub"
    lines = path.read_text().splitlines()
    keys = [s.strip() for s in lines if re.fullmatch(r"RW[A-Za-z0-9+/=]{54}", s.strip())]
    if len(keys) != 1 or keys[0] == UPSTREAM_KEY:
        raise ValueError("Configure an independent fork public key in packaging/update-key.pub first; see docs/UPDATES.md")
    return path


def verify_signature(package, signature, key):
    lines = signature.read_text().splitlines()
    if len(lines) != 4 or lines[2] != f"trusted comment: {package.name}":
        raise ValueError(f"Signature must bind the exact package name: {package.name}")
    run("minisign", "-V", "-q", "-p", key, "-m", package, "-x", signature)


def prepare(items, output, secret):
    key = public_key()
    if shutil.which("minisign") is None:
        raise ValueError("Install minisign before preparing release assets")
    output.mkdir(parents=True, exist_ok=False)
    for platform, item in items.items():
        source = item["path"]
        package = output / source.name
        shutil.copyfile(source, package)
        signature = output / (source.name + ".minisig")
        existing = Path(str(source) + ".minisig")
        if existing.exists():
            shutil.copyfile(existing, signature)
        else:
            if secret is None or not secret.is_file():
                raise ValueError(f"Supply --key or a detached .minisig for {source.name}")
            run("minisign", "-S", "-s", secret, "-m", package, "-x", signature, "-t", source.name)
        verify_signature(package, signature, key)
        if digest(package) != item["sha256"]:
            raise ValueError(f"Package changed while preparing: {source}")
        (output / (source.name + ".sha256")).write_text(f'{item["sha256"]}  {source.name}\n')
        manifest = {name: item[name] for name in ("version", "url", "size")}
        manifest["signature"] = signature.read_text()
        (output / f"update-{platform}.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return sorted(output.iterdir())


def verify_downloads(output, downloaded):
    expected = {path.name: path for path in output.iterdir() if path.is_file()}
    actual = {path.name: path for path in downloaded.iterdir() if path.is_file()}
    if actual.keys() != expected.keys():
        raise ValueError("Uploaded release assets are incomplete or contain unexpected files")
    for name, path in expected.items():
        if path.stat().st_size != actual[name].stat().st_size or digest(path) != digest(actual[name]):
            raise ValueError(f"Uploaded asset differs: {name}")
    key = public_key()
    for pattern in PLATFORMS.values():
        package = downloaded / pattern.format(version=version())
        verify_signature(package, downloaded / (package.name + ".minisig"), key)


def https_download(url, path, limit):
    class HTTPSOnly(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            if not newurl.startswith("https://"):
                raise ValueError("Release download attempted a non-HTTPS redirect")
            return super().redirect_request(req, fp, code, msg, headers, newurl)
    opener = urllib.request.build_opener(HTTPSOnly())
    with opener.open(url, timeout=60) as response, path.open("wb") as stream:
        count = 0
        for chunk in iter(lambda: response.read(1024 * 1024), b""):
            count += len(chunk)
            if count > limit:
                raise ValueError(f"Download exceeds expected size: {url}")
            stream.write(chunk)


def publish(output, assets, release, notes):
    if shutil.which("gh") is None:
        raise ValueError("Install and authenticate gh before publishing")
    if run("git", "status", "--porcelain", capture=True):
        raise ValueError("Publish from a clean checkout of the release commit")
    tag = f"v{release}"
    commit = run("git", "rev-parse", "HEAD", capture=True)
    if run("git", "rev-list", "-n", "1", tag, capture=True) != commit:
        raise ValueError("The release tag must already exist and point at HEAD")
    if run("gh", "api", f"repos/{REPO}/commits/{tag}", "--jq", ".sha", capture=True) != commit:
        raise ValueError("The remote release tag must also point at HEAD")
    run(sys.executable, ROOT / "scripts/check.py")
    # Do not update/overwrite an existing release or its assets.
    run("gh", "release", "create", tag, "--repo", REPO, "--verify-tag", "--draft", "--title", tag, "--notes-file", notes)
    run("gh", "release", "upload", tag, "--repo", REPO, *assets)
    with tempfile.TemporaryDirectory(prefix="quiet-release-check-") as temp:
        downloaded = Path(temp)
        run("gh", "release", "download", tag, "--repo", REPO, "--dir", downloaded)
        verify_downloads(output, downloaded)
    # Everything is present and verified before clients can discover this release.
    run("gh", "release", "edit", tag, "--repo", REPO, "--draft=false", "--prerelease=false", "--latest")
    with tempfile.TemporaryDirectory(prefix="quiet-public-check-") as temp:
        downloaded = Path(temp)
        for asset in assets:
            https_download(f"{BASE}/download/{tag}/{asset.name}", downloaded / asset.name, asset.stat().st_size)
        verify_downloads(output, downloaded)
        for platform in PLATFORMS:
            name = f"update-{platform}.json"
            path = downloaded / name
            https_download(f"{BASE}/latest/download/{name}", path, 65536)
            if path.read_bytes() != (output / name).read_bytes():
                raise ValueError(f"Latest manifest does not match: {name}")
    print(f"Published and verified: {BASE}/tag/{tag}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="+", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--dry-run", action="store_true", help="Inspect local packages; no signing, writes or network")
    mode.add_argument("--prepare", type=Path, metavar="NEW_DIRECTORY", help="Prepare assets locally without uploading")
    mode.add_argument("--publish", type=Path, metavar="NEW_DIRECTORY", help="Prepare assets, upload a verified draft, then publish")
    parser.add_argument("--key", type=Path, default=Path(os.environ["MINISIGN_KEY"]) if "MINISIGN_KEY" in os.environ else None)
    parser.add_argument("--notes", type=Path, help="Release notes; required for --publish")
    args = parser.parse_args()
    release = version()
    items = inventory(args.packages, release)
    if args.dry_run:
        print(json.dumps({p: {k: str(v) if isinstance(v, Path) else v for k, v in item.items()} for p, item in items.items()}, indent=2))
        return
    if args.publish and (args.notes is None or not args.notes.is_file()):
        raise ValueError("--publish requires --notes pointing to a release-notes file")
    output = (args.publish or args.prepare).resolve()
    assets = prepare(items, output, args.key.expanduser().resolve() if args.key else None)
    print(f"Prepared {len(assets)} assets in {output}")
    if args.publish:
        publish(output, assets, release, args.notes.resolve())


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"Release stopped: {error}")
