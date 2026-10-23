#!/usr/bin/env python3
"""Offline clean-HOME install/extract/doctor/real-Bash/disable/uninstall gate.

Run after cargo build/test have populated the local cache. Never changes profiles
or uses the real HOME, a provider, or a package registry.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix="reedline-installed-bash-") as temporary:
        home = Path(temporary)
        env = os.environ.copy()
        env.update(HOME=str(home), XDG_CACHE_HOME=str(home / "cache"),
                   XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"))
        env.pop("BASH_ENV", None)
        env.pop("ENV", None)
        env.pop("REEDLINE_AI_API_KEY", None)
        destination = home / "cargo-root"
        subprocess.run(["cargo", "install", "--path", str(ROOT), "--root", str(destination),
                        "--debug", "--locked", "--offline"], cwd=home, env=env, check=True)
        binary = destination / "bin/reedline-ai"
        assert binary.is_file()
        subprocess.run([str(binary), "adapter", "doctor", "--shell", "bash"], cwd=home, env=env, check=True)
        assets = home / "adapter with spaces"
        subprocess.run([str(binary), "adapter", "install", "--shell", "bash", "--directory", str(assets)],
                       cwd=home, env=env, check=True)
        env["RAI_BASH_ADAPTER_PATH"] = str(assets / "reedline-ai.bash")
        subprocess.run([sys.executable, str(ROOT / "scripts/pty_bash_adapter.py"), str(binary)],
                       cwd=home, env=env, check=True)
        subprocess.run([str(binary), "adapter", "uninstall", "--shell", "bash", "--directory", str(assets)],
                       cwd=home, env=env, check=True)
        assert not assets.exists()
        assert not any((home / name).exists() for name in (".bashrc", ".bash_profile", ".profile", ".bash_history"))
        print("Installed Bash PASS: offline cargo install, bundled assets without runtime checkout, doctor, real PTY/disable, uninstall; isolated HOME unchanged")


if __name__ == "__main__":
    main()
