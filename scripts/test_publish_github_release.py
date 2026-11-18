#!/usr/bin/env python3
"""Release safety checks; all packages and uploads are isolated test fixtures."""
import importlib.util
import json
import os
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
        unexpected = self.root / "unexpected.zip"
        unexpected.write_bytes(b"unexpected build")
        with self.assertRaisesRegex(ValueError, "Unexpected"):
            release.inventory([unexpected, *self.packages[1:]], self.version)

    def test_bad_checksum_is_rejected(self):
        Path(str(self.packages[0]) + ".sha256").write_text("wrong checksum\n")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
            release.inventory(self.packages, self.version)

    def test_publish_cli_accepts_optional_notes(self):
        notes = self.root / "notes.md"
        notes.write_text("Release notes")
        output = self.root / "prepared"
        for selected in [None, notes]:
            command = ["publish-github-release.py", "--publish", str(output), "--draft"]
            if selected is not None:
                command.extend(["--notes", str(selected)])
            command.extend(map(str, self.packages))
            with patch.object(sys, "argv", command), patch.object(release, "prepare", return_value=[]) as prepare, \
                    patch.object(release, "publish") as publish:
                release.main()
            prepare.assert_called_once()
            publish.assert_called_once_with(output.resolve(), [], self.version,
                                            selected.resolve() if selected is not None else None, draft=True)

    def test_missing_or_corrupt_uploaded_asset_keeps_release_draft(self):
        output = self.root / "assets"
        output.mkdir()
        asset = output / "package.zip"
        asset.write_text("expected package")
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
                    (destination / asset.name).write_text("tampered package")

            with patch.object(release, "run", side_effect=run), patch.object(release.shutil, "which", return_value="gh"):
                with self.assertRaises(ValueError):
                    release.publish(output, [asset], self.version)
            self.assertTrue(any(c[:3] == ("gh", "release", "create") and "--draft" in c for c in commands))
            self.assertFalse(any(c[:3] == ("gh", "release", "edit") for c in commands))


    def test_unsigned_cli_prepares_only_packages_and_checksums(self):
        packages = []
        for platform, pattern in release.PLATFORMS.items():
            path = self.root / pattern.format(version=self.version)
            path.write_bytes(platform.encode())
            packages.append(path)
        before = {p.name: p.read_bytes() for p in self.root.iterdir()}
        command = [sys.executable, release.__file__]
        plan = subprocess.run([*command, "--dry-run", *map(str, packages)], check=True, capture_output=True, text=True)
        self.assertEqual(set(json.loads(plan.stdout)), set(release.PLATFORMS))
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.root.iterdir()})
        output = self.root / "test-assets"
        subprocess.run([*command, "--prepare", str(output), *map(str, packages)], check=True, capture_output=True, text=True,
                       env={**os.environ, "PYTHONIOENCODING": "cp1252"})
        self.assertEqual({p.name for p in output.iterdir()}, {name for p in packages for name in (p.name, p.name + ".sha256")})
        for package in packages:
            self.assertEqual((output / package.name).read_bytes(), package.read_bytes())
            self.assertEqual((output / (package.name + ".sha256")).read_text(), f"{release.digest(package)}  {package.name}\n")
        with self.assertRaisesRegex(ValueError, "All four"):
            release.inventory(packages[:-1], self.version)
        Path(str(packages[0]) + ".sha256").write_text("wrong checksum")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
            release.inventory(packages, self.version)

    def test_publish_verifies_assets_before_making_stable_release_public(self):
        packages = []
        for platform, pattern in release.PLATFORMS.items():
            path = self.root / pattern.format(version=self.version)
            path.write_bytes(platform.encode())
            packages.append(path)
        output = self.root / "test-assets"
        assets = release.prepare(release.inventory(packages, self.version), output)
        notes = self.root / "notes.md"
        notes.write_text("Test version changes")
        for draft, corrupt in [(True, False), (False, False), (False, True)]:
            commands = []

            def run(*args, capture=False):
                commands.append(tuple(map(str, args)))
                if args[:3] == ("git", "status", "--porcelain"):
                    return ""
                if args[:3] == ("gh", "release", "view"):
                    return json.dumps({"tagName": f"v{self.version}", "isDraft": False, "isPrerelease": False})
                if capture:
                    return "release-commit"
                if args[:3] == ("gh", "release", "download"):
                    destination = Path(args[args.index("--dir") + 1])
                    shutil.copytree(output, destination, dirs_exist_ok=True)
                    if corrupt:
                        (destination / assets[0].name).write_bytes(b"corrupt")

            def download(url, path, limit):
                self.assertNotIn("/latest/", url)
                shutil.copyfile(output / path.name, path)

            with patch.object(release, "run", side_effect=run), patch.object(release.shutil, "which", return_value="gh"), \
                    patch.object(release, "https_download", side_effect=download) as public_download:
                if corrupt:
                    with self.assertRaisesRegex(ValueError, "differs"):
                        release.publish(output, assets, self.version, notes, draft=draft)
                else:
                    release.publish(output, assets, self.version, notes, draft=draft)
            create = next(c for c in commands if c[:3] == ("gh", "release", "create"))
            self.assertIn("--prerelease=false", create)
            self.assertIn("--latest=false", create)
            self.assertEqual(create[create.index("--title") + 1], f"v{self.version}")
            body = create[create.index("--notes") + 1]
            self.assertIn("ad-hoc", body)
            self.assertIn(notes.read_text(), body)
            edits = [c for c in commands if c[:3] == ("gh", "release", "edit")]
            self.assertEqual(len(edits), int(not draft and not corrupt))
            if edits:
                self.assertIn("--prerelease=false", edits[0])
                self.assertIn("--latest=true", edits[0])
            self.assertEqual(public_download.call_count, len(assets) if not draft and not corrupt else 0)
        with self.assertRaises(FileExistsError):
            release.prepare(release.inventory(packages, self.version), output)

    def test_publish_rejects_draft_or_prerelease_metadata(self):
        output = self.root / "assets"
        assets = release.prepare(release.inventory(self.packages, self.version), output)
        for field, value in [("isDraft", True), ("isPrerelease", True), ("tagName", "v0.0.0")]:
            state = {"tagName": f"v{self.version}", "isDraft": False, "isPrerelease": False, field: value}

            def run(*args, capture=False):
                if args[:3] == ("git", "status", "--porcelain"):
                    return ""
                if args[:3] == ("gh", "release", "view"):
                    return json.dumps(state)
                if capture:
                    return "release-commit"
                if args[:3] == ("gh", "release", "download"):
                    shutil.copytree(output, Path(args[args.index("--dir") + 1]), dirs_exist_ok=True)

            with patch.object(release, "run", side_effect=run), patch.object(release.shutil, "which", return_value="gh"), \
                    patch.object(release, "https_download") as download:
                with self.assertRaisesRegex(ValueError, "public and not a prerelease"):
                    release.publish(output, assets, self.version)
            download.assert_not_called()


if __name__ == "__main__":
    unittest.main()
