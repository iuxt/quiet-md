#!/usr/bin/env python3
"""Exercise version commands in a temporary checkout, without changing this one."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
FILES = ("VERSION", "Cargo.toml", "crates/core/Cargo.toml", "Cargo.lock")


class VersionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="quiet version 中文 ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name in (*FILES, "scripts/version.py"):
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.version = (self.root / "VERSION").read_text().strip()

    def run_command(self, *args):
        return subprocess.run([sys.executable, str(self.root / "scripts/version.py"), *args],
                              cwd=self.root.parent, capture_output=True, text=True)

    def snapshot(self):
        return {name: (self.root / name).read_bytes() for name in FILES}

    def test_get_and_invalid_arguments_are_read_only(self):
        before = self.snapshot()
        for args in [(), ("get",)]:
            result = self.run_command(*args)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, self.version + "\n")
        for args in [("bump",), ("get", "extra")]:
            self.assertNotEqual(self.run_command(*args).returncode, 0)
        self.assertEqual(self.snapshot(), before)

    def test_bumps_sync_only_the_workspace_versions(self):
        # Nonzero minor/patch components make resets observable for every bump.
        templates = {
            "VERSION": b"@VERSION@\n",
            "Cargo.toml": b'[package]\nname = "quiet_md"\nversion = "@VERSION@"\n[dependencies]\nfoo = "2.10.0"\n',
            "crates/core/Cargo.toml": b'[package]\nname = "fdl_core"\nversion = "@VERSION@"\n',
            "Cargo.lock": b'# lockfile\nversion = 4\n\n[[package]]\nname = "fdl_core"\nversion = "@VERSION@"\n'
                          b'\n[[package]]\nname = "quiet_md"\nversion = "@VERSION@"\n'
                          b'\n[[package]]\nname = "other"\nversion = "2.10.0"\n',
        }
        for name, contents in templates.items():
            (self.root / name).write_bytes(contents.replace(b"@VERSION@", b"2.9.7"))
        for command, expected in [("bump-minor", "2.10.0"), ("bump-patch", "2.10.1"), ("bump-major", "3.0.0")]:
            result = self.run_command(command)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, expected + "\n")
            for name, contents in self.snapshot().items():
                self.assertEqual(contents, templates[name].replace(b"@VERSION@", expected.encode()))
            self.assertEqual(self.run_command("get").stdout, expected + "\n")

    def test_invalid_or_inconsistent_versions_never_write(self):
        original = self.snapshot()
        for name, contents in [("VERSION", b"01.2.3\n"), ("VERSION", b"1.2.3-beta\n"),
                               ("crates/core/Cargo.toml", b'[package]\nversion = "99.0.0"\n'),
                               ("Cargo.lock", b"# missing workspace packages\n")]:
            for path, data in original.items():
                (self.root / path).write_bytes(data)
            (self.root / name).write_bytes(contents)
            before = self.snapshot()
            result = self.run_command("bump-patch")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("version:", result.stderr)
            self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
