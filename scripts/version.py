#!/usr/bin/env python3
"""Read/bump workspace versions, or push the version tag for a manual release."""
import argparse
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
SEMVER = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
MANIFEST = r'(?m)^version = "([^"\r\n]+)"'


def version_files(root):
    version = (root / "VERSION").read_text(encoding="utf-8").strip()
    if not re.fullmatch(SEMVER, version):
        raise ValueError("VERSION must contain MAJOR.MINOR.PATCH without leading zeroes")
    files = []
    for name, patterns in [
        ("Cargo.toml", [MANIFEST]),
        ("crates/core/Cargo.toml", [MANIFEST]),
        ("Cargo.lock", [rf'(?m)^name = "{package}"\r?\nversion = "([^"\r\n]+)"'
                        for package in ("quiet_md", "fdl_core")]),
    ]:
        path = root / name
        contents = path.read_bytes().decode("utf-8")
        spans = []
        for pattern in patterns:
            matches = list(re.finditer(pattern, contents))
            if len(matches) != 1:
                raise ValueError(f"Expected exactly one version matching {pattern!r} in {name}")
            match = matches[0]
            if match[1] != version:
                raise ValueError(f"{name} has version {match[1]}, but VERSION says {version}")
            spans.append(match.span(1))
        files.append((path, contents, spans))
    return version, files


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def release_tag(version):
    if git("status", "--porcelain"):
        raise ValueError("Release from a clean checkout; commit and push the version changes first")
    if git("symbolic-ref", "--short", "HEAD") != "main":
        raise ValueError("Release from the main branch")
    commit = git("rev-parse", "HEAD")
    tag = f"v{version}"
    ref = f"refs/tags/{tag}"
    remote = {}
    for line in git("ls-remote", "origin", "refs/heads/main", ref, ref + "^{}").splitlines():
        sha, name = line.split()
        remote[name] = sha
    if remote.get("refs/heads/main") != commit:
        raise ValueError("Push the release commit to origin/main before releasing")
    local = git("tag", "--list", tag)
    if local and git("rev-parse", ref + "^{commit}") != commit:
        raise ValueError(f"Existing local tag {tag} does not point at HEAD; tags are never moved")
    if ref in remote:
        if remote.get(ref + "^{}", remote[ref]) != commit:
            raise ValueError(f"Existing remote tag {tag} does not point at HEAD; tags are never moved")
        if not local:
            git("fetch", "origin", f"{ref}:{ref}")
        return tag
    if not local:
        git("tag", "-a", tag, "-m", f"Release {tag}")
    git("push", "origin", f"{ref}:{ref}")
    return tag


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs="?", default="get",
                        choices=("get", "bump-major", "bump-minor", "bump-patch", "release"))
    args = parser.parse_args()
    try:
        version, files = version_files(ROOT)
        if args.command == "release":
            print(release_tag(version))
            return
        if args.command != "get":
            numbers = list(map(int, version.split(".")))
            index = ("bump-major", "bump-minor", "bump-patch").index(args.command)
            numbers[index] += 1
            numbers[index + 1:] = [0] * (2 - index)
            version = ".".join(map(str, numbers))
            # Validate every file before changing any; preserve all unrelated bytes.
            for path, contents, spans in files:
                for start, end in sorted(spans, reverse=True):
                    contents = contents[:start] + version + contents[end:]
                path.write_bytes(contents.encode("utf-8"))
            (ROOT / "VERSION").write_bytes((version + "\n").encode("utf-8"))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        if isinstance(error, subprocess.CalledProcessError):
            error = error.stderr.strip() or str(error)
        parser.exit(1, f"version: {error}\n")
    print(version)


if __name__ == "__main__":
    main()
