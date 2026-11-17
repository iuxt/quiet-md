#!/usr/bin/env python3
"""Read VERSION or bump it together with both Cargo manifests and Cargo.lock."""
import argparse
from pathlib import Path
import re

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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs="?", default="get",
                        choices=("get", "bump-major", "bump-minor", "bump-patch"))
    args = parser.parse_args()
    try:
        version, files = version_files(ROOT)
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
            (ROOT / "VERSION").write_text(version + "\n", encoding="utf-8")
    except (OSError, ValueError) as error:
        parser.exit(1, f"version: {error}\n")
    print(version)


if __name__ == "__main__":
    main()
