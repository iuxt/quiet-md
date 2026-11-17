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

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True, text=True).stdout.strip()

    def init_repository(self):
        self.remote = self.root.parent / (self.root.name + "-remote.git")
        self.addCleanup(shutil.rmtree, self.remote, True)
        self.git("init", "--bare", "--template=", str(self.remote))
        self.git("init", "--template=", "-b", "main")
        for key, value in [("user.name", "Version test"), ("user.email", "version@example.invalid"),
                           ("commit.gpgSign", "false"), ("tag.gpgSign", "false"),
                           ("core.hooksPath", str(self.root / ".git/no-hooks"))]:
            self.git("config", key, value)
        self.git("add", ".")
        self.git("commit", "-m", "Release source")
        self.git("remote", "add", "origin", str(self.remote))
        self.git("push", "origin", "main")

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

    def test_release_pushes_current_version_and_can_be_retried(self):
        self.init_repository()
        before = self.snapshot()
        tag = "v" + self.version
        for attempt in range(3):
            if attempt == 2:
                self.git("tag", "-d", tag)
            result = self.run_command("release")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, tag + "\n")
            self.assertEqual(self.git("rev-parse", tag + "^{commit}"), self.git("rev-parse", "HEAD"))
            remote = self.git("ls-remote", "origin", f"refs/tags/{tag}^{{}}")
            self.assertEqual(remote.split()[0], self.git("rev-parse", "HEAD"))
            self.assertEqual(self.snapshot(), before)
            self.assertEqual(self.git("status", "--porcelain"), "")

    def test_release_rejects_dirty_wrong_branch_and_unpushed_commits(self):
        self.init_repository()
        for name in ("VERSION", "untracked.txt"):
            path = self.root / name
            original = path.read_bytes() if path.exists() else None
            path.write_text(self.version + "\n\n" if original is not None else "untracked")
            result = self.run_command("release")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("clean checkout", result.stderr)
            if original is None:
                path.unlink()
            else:
                path.write_bytes(original)
        self.git("checkout", "-b", "feature")
        self.assertIn("main branch", self.run_command("release").stderr)
        self.git("checkout", "main")
        self.git("commit", "--allow-empty", "-m", "Unpushed source")
        self.assertIn("origin/main", self.run_command("release").stderr)
        self.assertEqual(self.git("tag", "--list"), "")
        self.assertEqual(self.git("ls-remote", "origin", "refs/tags/*"), "")

    def test_release_never_moves_conflicting_local_or_remote_tags(self):
        self.init_repository()
        tag = "v" + self.version
        old = self.git("rev-parse", "HEAD")
        self.git("tag", tag)
        self.git("commit", "--allow-empty", "-m", "New source")
        self.git("push", "origin", "main")
        result = self.run_command("release")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Existing local tag", result.stderr)
        self.assertEqual(self.git("rev-parse", tag), old)
        self.git("push", "origin", f"refs/tags/{tag}")
        self.git("tag", "-d", tag)
        result = self.run_command("release")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Existing remote tag", result.stderr)
        self.assertEqual(self.git("tag", "--list"), "")
        self.assertEqual(self.git("ls-remote", "origin", f"refs/tags/{tag}").split()[0], old)

    def test_release_reuses_matching_local_tag(self):
        self.init_repository()
        tag = "v" + self.version
        self.git("tag", tag)
        result = self.run_command("release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.git("cat-file", "-t", tag), "commit")
        self.assertEqual(self.git("ls-remote", "origin", f"refs/tags/{tag}").split()[0], self.git("rev-parse", "HEAD"))

    def test_failed_push_can_retry_without_recreating_tag(self):
        self.init_repository()
        hook = self.remote / "hooks/pre-receive"
        hook.parent.mkdir()
        hook.write_text("#!/bin/sh\nexit 1\n")
        hook.chmod(0o755)
        # The receiving repository must run its own hook, regardless of global config.
        self.git("--git-dir", str(self.remote), "config", "core.hooksPath", str(hook.parent))
        tag = "v" + self.version
        result = self.run_command("release")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("version:", result.stderr)
        original = self.git("rev-parse", tag)
        self.assertEqual(self.git("ls-remote", "origin", f"refs/tags/{tag}"), "")
        hook.unlink()
        result = self.run_command("release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.git("rev-parse", tag), original)
        self.assertEqual(self.git("ls-remote", "origin", f"refs/tags/{tag}").split()[0], original)


if __name__ == "__main__":
    unittest.main()
