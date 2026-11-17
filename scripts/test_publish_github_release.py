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

    def test_unsigned_cli_prepares_only_packages_and_checksums(self):
        packages = []
        for platform, pattern in release.TEST_PLATFORMS.items():
            path = self.root / pattern.format(version=self.version)
            path.write_bytes(platform.encode())
            packages.append(path)
        before = {p.name: p.read_bytes() for p in self.root.iterdir()}
        command = [sys.executable, release.__file__, "--test-release"]
        plan = subprocess.run([*command, "--dry-run", *map(str, packages)], check=True, capture_output=True, text=True)
        self.assertEqual(set(json.loads(plan.stdout)), set(release.PLATFORMS))
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.root.iterdir()})
        output = self.root / "test-assets"
        subprocess.run([*command, "--prepare", str(output), *map(str, packages)], check=True, capture_output=True, text=True)
        self.assertEqual({p.name for p in output.iterdir()}, {name for p in packages for name in (p.name, p.name + ".sha256")})
        for package in packages:
            self.assertEqual((output / package.name).read_bytes(), package.read_bytes())
            self.assertEqual((output / (package.name + ".sha256")).read_text(), f"{release.digest(package)}  {package.name}\n")
        with self.assertRaisesRegex(ValueError, "All four"):
            release.inventory(packages[:-1], self.version, test_release=True)
        Path(str(packages[0]) + ".sha256").write_text("wrong checksum")
        with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
            release.inventory(packages, self.version, test_release=True)

    def test_unsigned_publish_never_creates_update_manifests_or_latest_release(self):
        packages = []
        for platform, pattern in release.TEST_PLATFORMS.items():
            path = self.root / pattern.format(version=self.version)
            path.write_bytes(platform.encode())
            packages.append(path)
        output = self.root / "test-assets"
        assets = release.prepare_test(release.inventory(packages, self.version, test_release=True), output)
        notes = self.root / "notes.md"
        notes.write_text("Test version changes")
        for draft, corrupt in [(True, False), (False, False), (False, True)]:
            commands = []

            def run(*args, capture=False):
                commands.append(tuple(map(str, args)))
                if args[:3] == ("git", "status", "--porcelain"):
                    return ""
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
                    patch.object(release, "public_key", side_effect=AssertionError("Unsigned releases never read signing keys")), \
                    patch.object(release, "https_download", side_effect=download) as public_download:
                if corrupt:
                    with self.assertRaisesRegex(ValueError, "differs"):
                        release.publish(output, assets, self.version, notes, draft=draft, test_release=True)
                else:
                    release.publish(output, assets, self.version, notes, draft=draft, test_release=True)
            create = next(c for c in commands if c[:3] == ("gh", "release", "create"))
            self.assertIn("--prerelease", create)
            self.assertIn("--latest=false", create)
            body = create[create.index("--notes") + 1]
            self.assertIn("ad-hoc", body)
            self.assertIn(notes.read_text(), body)
            edits = [c for c in commands if c[:3] == ("gh", "release", "edit")]
            self.assertEqual(len(edits), int(not draft and not corrupt))
            if edits:
                self.assertIn("--prerelease=true", edits[0])
                self.assertIn("--latest=false", edits[0])
            self.assertEqual(public_download.call_count, len(assets) if not draft and not corrupt else 0)
        self.assertFalse(any(p.name.startswith("update-") for p in output.iterdir()))
        with self.assertRaises(FileExistsError):
            release.prepare_test(release.inventory(packages, self.version, test_release=True), output)

    def test_verified_draft_is_not_made_latest(self):
        output = self.root / "assets"
        output.mkdir()
        for platform, item in release.inventory(self.packages, self.version).items():
            source = item["path"]
            shutil.copyfile(source, output / source.name)
            (output / (source.name + ".minisig")).write_text("test signature")
            (output / (source.name + ".sha256")).write_text(item["sha256"])
            (output / f"update-{platform}.json").write_text(json.dumps({"version": self.version}))
        for draft in [True, False]:
            notes = self.root / "notes.md" if draft else None
            commands = []

            def run(*args, capture=False):
                commands.append(tuple(map(str, args)))
                if args[:3] == ("git", "status", "--porcelain"):
                    return ""
                if capture:
                    return "release-commit"
                if args[:3] == ("gh", "release", "download"):
                    destination = Path(args[args.index("--dir") + 1])
                    shutil.copytree(output, destination, dirs_exist_ok=True)

            def download(url, path, limit):
                shutil.copyfile(output / path.name, path)

            with patch.object(release, "run", side_effect=run), patch.object(release.shutil, "which", return_value="gh"), \
                    patch.object(release, "public_key", return_value=self.root / "key.pub"), patch.object(release, "verify_signature") as verify, \
                    patch.object(release, "https_download", side_effect=download) as public_download:
                release.publish(output, sorted(output.iterdir()), self.version, notes, draft=draft)
            self.assertEqual(verify.call_count, 4 if draft else 8)
            self.assertEqual(public_download.call_count, 0 if draft else 20)
            self.assertEqual(any(c[:3] == ("gh", "release", "edit") for c in commands), not draft)
            create = next(c for c in commands if c[:3] == ("gh", "release", "create"))
            self.assertEqual("--generate-notes" in create, notes is None)
            self.assertEqual("--notes-file" in create, notes is not None)
            if notes is not None:
                self.assertEqual(create[create.index("--notes-file") + 1], str(notes))

    @unittest.skipUnless(shutil.which("minisign"), "minisign is required for the encrypted-key integration test")
    def test_encrypted_key_signs_all_platforms_without_a_terminal(self):
        key = self.root / "temporary.key"
        public = self.root / "temporary.pub"
        password = "quiet-md-test-password"
        subprocess.run(["minisign", "-G", "-p", str(public), "-s", str(key)],
                       input=f"{password}\n{password}\n", text=True, capture_output=True, check=True, timeout=30)
        with patch.dict(os.environ, {"MINISIGN_PASSWORD": password}), patch.object(release, "public_key", return_value=public):
            output = self.root / "signed"
            release.prepare(release.inventory(self.packages, self.version), output, key)
            copied = self.root / "downloaded"
            shutil.copytree(output, copied)
            release.verify_downloads(output, copied)
            signature = copied / (self.packages[0].name + ".minisig")
            signature.write_text(signature.read_text().replace(self.packages[0].name, "another-version.zip"))
            with self.assertRaisesRegex(ValueError, "exact package name"):
                release.verify_signature(copied / self.packages[0].name, signature, public)


if __name__ == "__main__":
    unittest.main()
