#!/usr/bin/env python3
"""Release safety checks; all packages and uploads are isolated test fixtures."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("publish", Path(__file__).with_name("publish-github-release.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="quiet release 中文 ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.version = release.version()
        self.packages = []
        for platform, pattern in release.PLATFORMS.items():
            path = self.root / pattern.format(version=self.version)
            path.write_bytes(platform.encode())
            self.packages.append(path)

    def test_dry_run_is_read_only_and_requires_all_platforms(self):
        before = {p.name: p.read_bytes() for p in self.root.iterdir()}
        result = subprocess.run([sys.executable, str(Path(release.__file__)), "--dry-run", *map(str, self.packages)],
                                check=True, capture_output=True, text=True)
        plan = json.loads(result.stdout)
        self.assertEqual(set(plan), set(release.PLATFORMS))
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.root.iterdir()})
        with self.assertRaisesRegex(ValueError, "All four"):
            release.inventory(self.packages[:-1], self.version)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            release.inventory(self.packages + self.packages[:1], self.version)
        unsigned = self.root / self.packages[0].name.replace(".zip", "-unsigned.zip")
        unsigned.write_bytes(b"validation build")
        with self.assertRaisesRegex(ValueError, "Unexpected"):
            release.inventory([unsigned, *self.packages[1:]], self.version)

    def test_bad_checksum_and_upstream_key_are_rejected(self):
        Path(str(self.packages[0]) + ".sha256").write_text("wrong checksum\n")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
            release.inventory(self.packages, self.version)
        (self.root / "packaging").mkdir()
        key = self.root / "packaging/update-key.pub"
        for contents in ["unconfigured", release.UPSTREAM_KEY]:
            key.write_text(contents)
            with patch.object(release, "ROOT", self.root), self.assertRaisesRegex(ValueError, "independent"):
                release.public_key()

    def test_signature_must_bind_the_exact_name(self):
        signature = self.root / "wrong.minisig"
        signature.write_text("untrusted comment: test\nsignature\ntrusted comment: another-version.zip\nglobal\n")
        with patch.object(release, "run") as run, self.assertRaisesRegex(ValueError, "exact package name"):
            release.verify_signature(self.packages[0], signature, self.root / "key.pub")
        run.assert_not_called()

    def test_missing_or_corrupt_uploaded_asset_keeps_release_draft(self):
        output = self.root / "assets"
        output.mkdir()
        asset = output / "update-macos-arm64.json"
        asset.write_text("expected manifest")
        for corrupt in [False, True]:
            commands = []

            def run(*args, capture=False):
                commands.append(tuple(map(str, args)))
                if args[:3] == ("git", "status", "--porcelain"):
                    return ""
                if capture:
                    return "release-commit"
                if args[:3] == ("gh", "release", "download") and corrupt:
                    destination = Path(args[args.index("--dir") + 1])
                    (destination / asset.name).write_text("tampered manifest")

            with patch.object(release, "run", side_effect=run), patch.object(release.shutil, "which", return_value="gh"):
                with self.assertRaises(ValueError):
                    release.publish(output, [asset], self.version, self.root / "notes.md")
            self.assertTrue(any(c[:3] == ("gh", "release", "create") and "--draft" in c for c in commands))
            self.assertFalse(any(c[:3] == ("gh", "release", "edit") for c in commands))

    def test_preparation_emits_complete_manifests_and_never_overwrites(self):
        items = release.inventory(self.packages, self.version)
        for source in self.packages:
            Path(str(source) + ".minisig").write_text(f"untrusted comment: test\nsignature\ntrusted comment: {source.name}\nglobal\n")
        output = self.root / "prepared"
        with patch.object(release, "public_key", return_value=self.root / "key.pub"), \
                patch.object(release.shutil, "which", return_value="minisign"), patch.object(release, "run"):
            assets = release.prepare(items, output, None)
            self.assertEqual(len(assets), 16)
            for platform, item in items.items():
                manifest = json.loads((output / f"update-{platform}.json").read_text())
                self.assertEqual(manifest["url"], item["url"])
                self.assertEqual(manifest["size"], item["path"].stat().st_size)
                self.assertIn(f'trusted comment: {item["path"].name}', manifest["signature"])
            with self.assertRaises(FileExistsError):
                release.prepare(items, output, None)
        copied = self.root / "downloaded"
        shutil.copytree(output, copied)
        package = copied / self.packages[0].name
        package.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "differs"):
            release.verify_downloads(output, copied)


if __name__ == "__main__":
    unittest.main()
