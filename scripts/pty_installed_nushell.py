#!/usr/bin/env python3
"""Clean-HOME installed experimental Nushell gate. Usage: SCRIPT /path/to/nu.
Requires populated Cargo cache; never changes user profiles or calls providers.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
ROOT = Path(__file__).resolve().parents[1]
NU = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="reedline-installed-nu-") as temporary:
    home = Path(temporary)
    env = dict(os.environ, HOME=str(home), XDG_CACHE_HOME=str(home / "cache"),
               XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"),
               PYTHONDONTWRITEBYTECODE="1")
    env.pop("REEDLINE_AI_API_KEY", None)
    env["PATH"] = str(NU.parent) + os.pathsep + env["PATH"]
    root = home / "cargo-root"
    subprocess.run(["cargo", "install", "--path", str(ROOT), "--root", str(root),
                    "--debug", "--locked", "--offline"], cwd=home, env=env, check=True)
    binary = root / "bin/reedline-ai"
    subprocess.run([str(binary), "adapter", "doctor", "--shell", "nushell"], cwd=home, env=env, check=True)
    assets = home / "Nu adapter with spaces"
    subprocess.run([str(binary), "adapter", "install", "--shell", "nushell", "--experimental",
                    "--directory", str(assets)], cwd=home, env=env, check=True)
    env.update(NU_BINARY=str(NU), REEDLINE_AI_TEST_BINARY=str(binary),
               RAI_NUSHELL_ADAPTER_PATH=str(assets / "nushell/reedline-ai.nu"),
               RAI_SHARED_BRIDGE_PATH=str(assets / "shared/bridge.py"))
    for suite in ("shared", "nushell"):
        subprocess.run([sys.executable, "-m", "unittest", "discover", "-s",
                        str(ROOT / "integrations" / suite), "-p", "test_*.py"], cwd=home, env=env, check=True)
    subprocess.run([str(binary), "adapter", "uninstall", "--shell", "nushell",
                    "--directory", str(assets)], cwd=home, env=env, check=True)
    assert not assets.exists()
    assert not (home / ".config/nushell/config.nu").exists()
    print("Installed Nushell PASS: clean-HOME offline install, doctor, bundled native adapter/helper, real mock/PTY/disable, uninstall. POSIX host only; Windows terminal validation remains separate.")
