#!/usr/bin/env python3
"""Prepare manual-install packages and checksums, or publish a verified prerelease.

Dry-run is read-only. --prepare writes eight assets into a new directory.
--publish additionally uploads and verifies a draft, then publishes it unless
--draft is selected. gh supplies publishing credentials.
"""
import argparse
import hashlib
import json
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
PLATFORMS = {
    "macos-arm64": "quiet-md-{version}-macos-arm64-unsigned.zip",
    "macos-x86_64": "quiet-md-{version}-macos-x86_64-unsigned.zip",
    "windows-x64": "quiet-md-{version}-windows-x64-setup.exe",
    "linux-x86_64": "quiet-md-{version}-x86_64.AppImage",
}
TEST_NOTICE = """**免费测试包 / Free test build**

macOS 包采用 ad-hoc 签名，未经 Apple 公证；Windows 安装包未签名，系统可能提示安全警告。
请手动下载和安装，并使用随包的 SHA-256 文件检查下载完整性。

The macOS apps are ad-hoc signed and not notarized by Apple. The Windows installer
is unsigned; your system may show security warnings. Download and install manually.
SHA-256 files check download integrity.
"""


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


def prepare(items, output):
    output.mkdir(parents=True, exist_ok=False)
    for item in items.values():
        source = item["path"]
        package = output / source.name
        shutil.copyfile(source, package)
        if digest(package) != item["sha256"]:
            raise ValueError(f"Package changed while preparing: {source}")
        (output / (source.name + ".sha256")).write_text(f'{item["sha256"]}  {source.name}\n')
    return sorted(output.iterdir())


def verify_downloads(output, downloaded):
    expected = {path.name: path for path in output.iterdir() if path.is_file()}
    actual = {path.name: path for path in downloaded.iterdir() if path.is_file()}
    if actual.keys() != expected.keys():
        raise ValueError("Uploaded release assets are incomplete or contain unexpected files")
    for name, path in expected.items():
        if path.stat().st_size != actual[name].stat().st_size or digest(path) != digest(actual[name]):
            raise ValueError(f"Uploaded asset differs: {name}")


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


def publish(output, assets, release, notes=None, draft=False):
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
    notes_args = ["--notes", TEST_NOTICE + ("\n" + notes.read_text() if notes is not None else ""),
                  *([] if notes is not None else ["--generate-notes"])]
    flags = ["--prerelease", "--latest=false"]
    title = f"{tag} — Test build (unsigned)"
    run("gh", "release", "create", tag, "--repo", REPO, "--verify-tag", "--draft", "--title", title, *flags, *notes_args)
    run("gh", "release", "upload", tag, "--repo", REPO, *assets)
    with tempfile.TemporaryDirectory(prefix="quiet-release-check-") as temp:
        downloaded = Path(temp)
        run("gh", "release", "download", tag, "--repo", REPO, "--dir", downloaded)
        verify_downloads(output, downloaded)
    if draft:
        print(f"Verified draft: {BASE}/tag/{tag}")
        return
    # Everything is present and verified before the release becomes public.
    flags = ["--prerelease=true", "--latest=false"]
    run("gh", "release", "edit", tag, "--repo", REPO, "--draft=false", *flags)
    with tempfile.TemporaryDirectory(prefix="quiet-public-check-") as temp:
        downloaded = Path(temp)
        for asset in assets:
            https_download(f"{BASE}/download/{tag}/{asset.name}", downloaded / asset.name, asset.stat().st_size)
        verify_downloads(output, downloaded)
    print(f"Published and verified: {BASE}/tag/{tag}")


def main():
    # Windows redirected stdout may use a legacy code page; paths can be Unicode.
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="backslashreplace")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="+", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--dry-run", action="store_true", help="Inspect local packages; no writes or network")
    mode.add_argument("--prepare", type=Path, metavar="NEW_DIRECTORY", help="Prepare assets locally without uploading")
    mode.add_argument("--publish", type=Path, metavar="NEW_DIRECTORY", help="Prepare assets, upload a verified draft, then publish")
    parser.add_argument("--draft", action="store_true", help="With --publish, leave the verified release as a draft")
    parser.add_argument("--notes", type=Path, help="Release notes file; omit to generate notes with GitHub")
    args = parser.parse_args()
    if args.draft and not args.publish:
        raise ValueError("--draft requires --publish")
    release = version()
    items = inventory(args.packages, release)
    if args.dry_run:
        print(json.dumps({p: {k: str(v) if isinstance(v, Path) else v for k, v in item.items()} for p, item in items.items()}, indent=2))
        return
    if args.publish and args.notes is not None and not args.notes.is_file():
        raise ValueError("--notes must point to a release-notes file")
    output = (args.publish or args.prepare).resolve()
    assets = prepare(items, output)
    print(f"Prepared {len(assets)} assets in {output}")
    if args.publish:
        publish(output, assets, release, args.notes.resolve() if args.notes is not None else None, draft=args.draft)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"Release stopped: {error}")
