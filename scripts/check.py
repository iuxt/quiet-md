#!/usr/bin/env python3
"""Shared macOS, Windows and Linux checks; run inside the platform build environment."""
import os
from pathlib import Path
import subprocess
import sys

os.chdir(Path(__file__).resolve().parent.parent)
def cargo(*args, env=None):
    subprocess.run(["cargo", *args], check=True, env=env)

subprocess.run([sys.executable, "scripts/version.py"], check=True)
subprocess.run([sys.executable, "scripts/test_version.py"], check=True)
subprocess.run([sys.executable, "scripts/test_publish_github_release.py"], check=True)
subprocess.run([sys.executable, "scripts/test_sign_local_macos.py"], check=True)
cargo("test", "--release", "--workspace", "--locked")
cargo("test", "--release", "--locked", "-p", "fdl_core", "library_round_trips", "--", "--ignored",
      env={**os.environ, "FDL_ROUNDTRIP_DIR": str(Path("shared-mock/library").resolve())})
source = Path("shared-mock/guides/assets")
bundled = Path("shared-mock/library")
for asset in source.rglob("*"):
    if asset.is_file() and not asset.name.startswith("."):
        targets = [bundled / folder / "guides" / asset.relative_to(source) for folder in ("Image", "Files", "Drawing")]
        found = [path for path in targets if path.is_file()]
        assert len(found) == 1 and found[0].read_bytes() == asset.read_bytes(), f"Bundled guide asset is stale: {asset}"
print("Bundled guide assets match their sources")
cargo("clippy", "--release", "--workspace", "--locked", "--all-targets", "--", "-D", "warnings")
# The harness build (script driver compiled in) must stay clean too.
cargo("clippy", "--release", "--workspace", "--locked", "--all-targets", "--features", "devtools", "--", "-D", "warnings")
