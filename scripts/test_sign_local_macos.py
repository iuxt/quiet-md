#!/usr/bin/env python3
"""Verify rebuild identity and signature isolation using temporary macOS app bundles."""
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

SIGNER = Path(__file__).resolve().parent / "sign-local-macos.py"


@unittest.skipUnless(sys.platform == "darwin", "macOS code signing")
class LocalSigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="quiet signing 中文 ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.addCleanup(self.remove_keychains)

    def remove_keychains(self):
        for keychain in self.root.rglob("*.keychain-db"):
            subprocess.run(["security", "delete-keychain", str(keychain)], check=True,
                           capture_output=True)

    def bundle(self, name, value):
        app = self.root / f"{name}.app"
        binary = app / "Contents/MacOS/quiet_md"
        binary.parent.mkdir(parents=True)
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps({
            "CFBundleIdentifier": "local.quietmd.signing-test", "CFBundleExecutable": "quiet_md",
            "CFBundlePackageType": "APPL", "CFBundleVersion": str(value),
        }))
        source = self.root / f"{name}.c"
        source.write_text(f"int main(void) {{ return {value}; }}\n")
        subprocess.run(["cc", str(source), "-o", str(binary)], check=True, capture_output=True)
        return app

    def sign(self, app, state):
        return subprocess.run([sys.executable, str(SIGNER), str(app)], capture_output=True, text=True,
                              env={**os.environ, "QUIET_MD_SIGNING_DIR": str(state)})

    def requirement(self, app):
        result = subprocess.run(["codesign", "-dr", "-", str(app)], check=True,
                                capture_output=True, text=True)
        return next(line.removeprefix("# ").removeprefix("designated => ")
                    for line in (result.stdout + result.stderr).splitlines() if "designated =>" in line)

    def test_changed_builds_keep_identity_and_reject_other_signers(self):
        first, second = self.bundle("first", 0), self.bundle("second", 1)
        state = self.root / "signing"
        result = self.sign(first, state)
        self.assertEqual(result.returncode, 0, result.stderr)
        certificate = (state / "certificate.pem").read_bytes()
        requirement = self.requirement(first)
        self.assertNotIn("cdhash", requirement)
        self.assertIn("certificate", requirement)
        result = self.sign(second, state)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((state / "certificate.pem").read_bytes(), certificate)
        self.assertEqual(self.requirement(second), requirement)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", "-R", "=" + requirement,
                        str(second)], check=True, capture_output=True)
        self.assertEqual(state.stat().st_mode & 0o777, 0o700)
        for name in ("password", "certificate.pem", "identity.keychain-db"):
            self.assertEqual((state / name).stat().st_mode & 0o077, 0)

        result = self.sign(second, self.root / "other-signing")
        self.assertEqual(result.returncode, 0, result.stderr)
        result = subprocess.run(["codesign", "--verify", "--strict", "-R", "=" + requirement,
                                 str(second)], capture_output=True)
        self.assertNotEqual(result.returncode, 0, "A different key must not inherit permissions")

        (state / "password").unlink()
        result = self.sign(first, state)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Incomplete signing identity", result.stderr)
        self.assertEqual((state / "certificate.pem").read_bytes(), certificate)
        self.assertEqual(self.requirement(first), requirement)


if __name__ == "__main__":
    unittest.main()
