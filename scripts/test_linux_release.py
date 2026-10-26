#!/usr/bin/env python3
"""Offline prebuilt archive lifecycle and refusal tests; no Rust build required."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("build_linux_release", ROOT / "scripts/build_linux_release.py")
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)
BINARY = Path(os.environ.get("REEDLINE_AI_RELEASE_BINARY", ROOT / "target/release/reedline-ai"))


@unittest.skipUnless(sys.platform.startswith("linux") and BINARY.is_file() and shutil.which("readelf"),
                     "native Linux release binary and readelf required")
class LinuxReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.shared = tempfile.TemporaryDirectory(prefix="rai-release-shared-")
        cls.archive = BUILDER.build(BINARY, Path(cls.shared.name))

    @classmethod
    def tearDownClass(cls):
        cls.shared.cleanup()

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rai-release-test-")
        self.home = Path(self.temporary.name)
        with tarfile.open(self.archive, "r:gz") as archive:
            # Archive was generated from an explicit allowlist above. Refuse
            # links and escaping names rather than trust extractor defaults.
            for item in archive.getmembers():
                self.assertTrue(item.isfile())
                self.assertFalse(item.name.startswith("/"))
                self.assertNotIn("..", Path(item.name).parts)
                target = self.home / item.name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(archive.extractfile(item).read())
                target.chmod(item.mode)
        self.package = self.home / self.archive.name.removesuffix(".tar.gz")
        self.prefix = self.home / "installed with spaces"
        self.env = os.environ.copy()
        self.env.update(HOME=str(self.home), XDG_CONFIG_HOME=str(self.home / "config"),
                        XDG_CACHE_HOME=str(self.home / "cache"), XDG_DATA_HOME=str(self.home / "data"))
        for name in ("REEDLINE_AI_API_KEY", "BASH_ENV", "ENV"):
            self.env.pop(name, None)

    def tearDown(self):
        self.temporary.cleanup()

    def command(self, action, success=True, package=None):
        result = subprocess.run([sys.executable, str((package or self.package) / "manage.py"),
                                 action, "--prefix", str(self.prefix)],
                                env=self.env, capture_output=True, text=True)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def test_deterministic_archive_and_checksum(self):
        rebuilt = BUILDER.build(BINARY, self.home / "rebuilt")
        self.assertEqual(self.archive.read_bytes(), rebuilt.read_bytes())
        digest = hashlib.sha256(rebuilt.read_bytes()).hexdigest()
        self.assertEqual((rebuilt.parent / (rebuilt.name + ".sha256")).read_text().split()[0], digest)
        manifest = json.loads((self.package / "manifest.json").read_text())
        self.assertEqual(manifest["architecture"], "x86_64" if os.uname().machine == "x86_64" else "aarch64")
        self.assertRegex(manifest["minimum_glibc"], r"^2\.\d+(\.\d+)?$")

    def test_install_mock_doctor_adapter_remove(self):
        self.command("install")
        binary = self.prefix / "bin/reedline-ai"
        completed = subprocess.run([str(binary), "--complete", "git st"], env=self.env,
                                   capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(completed.stdout)["result"], "git status")
        subprocess.run([str(binary), "adapter", "doctor", "--shell", "bash"], env=self.env,
                       capture_output=True, check=True)
        assets = self.home / "test adapter"
        subprocess.run([str(binary), "adapter", "install", "--shell", "bash", "--directory", str(assets)],
                       env=self.env, capture_output=True, check=True)
        subprocess.run([str(binary), "adapter", "uninstall", "--shell", "bash", "--directory", str(assets)],
                       env=self.env, capture_output=True, check=True)
        self.assertFalse(assets.exists())
        self.command("uninstall", package=self.prefix)
        self.assertFalse(self.prefix.exists())
        for name in (".bashrc", ".bash_profile", ".profile", ".bash_history", "config"):
            self.assertFalse((self.home / name).exists())

    def test_existing_install_not_overwritten(self):
        self.prefix.mkdir()
        marker = self.prefix / "personal.txt"
        marker.write_text("keep")
        self.command("install", success=False)
        self.assertEqual(marker.read_text(), "keep")

    def test_tampered_source_refused_before_install(self):
        (self.package / "LICENSE").write_text("altered")
        self.command("install", success=False)
        self.assertFalse(self.prefix.exists())

    def test_modified_install_refuses_all_removal(self):
        self.command("install")
        (self.prefix / "LICENSE").write_text("personal edit")
        self.command("uninstall", success=False)
        self.assertTrue((self.prefix / "bin/reedline-ai").is_file())
        self.assertEqual((self.prefix / "LICENSE").read_text(), "personal edit")

    def test_extra_file_refuses_all_removal(self):
        self.command("install")
        (self.prefix / "personal.txt").write_text("keep")
        self.command("uninstall", success=False)
        self.assertTrue((self.prefix / "bin/reedline-ai").is_file())

    def test_symlink_refused(self):
        self.command("install")
        original = (self.prefix / "LICENSE").read_bytes()
        outside = self.home / "outside.txt"
        outside.write_bytes(original)
        (self.prefix / "LICENSE").unlink()
        (self.prefix / "LICENSE").symlink_to(outside)
        self.command("uninstall", success=False)
        self.assertEqual(outside.read_bytes(), original)
        self.assertTrue((self.prefix / "bin/reedline-ai").is_file())

    def test_escaping_manifest_refused(self):
        manifest = json.loads((self.package / "manifest.json").read_text())
        manifest["files"]["../outside"] = "0" * 64
        (self.package / "manifest.json").write_text(json.dumps(manifest))
        self.command("install", success=False)
        self.assertFalse(self.prefix.exists())


if __name__ == "__main__":
    unittest.main()
