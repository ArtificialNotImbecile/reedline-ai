#!/usr/bin/env python3
"""Clean-HOME installed experimental PowerShell gate. Usage: SCRIPT /path/to/pwsh.

Requires a populated Cargo cache. No registry/API/profile activity.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
ROOT = Path(__file__).resolve().parents[1]
PWSH = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="reedline-installed-pwsh-") as temporary:
    home = Path(temporary)
    env = dict(os.environ, HOME=str(home), XDG_CACHE_HOME=str(home / "cache"),
               XDG_CONFIG_HOME=str(home / "config"), XDG_DATA_HOME=str(home / "data"))
    env.pop("REEDLINE_AI_API_KEY", None)
    env["PATH"] = str(PWSH.parent) + os.pathsep + env["PATH"]
    root = home / "cargo-root"
    subprocess.run(["cargo", "install", "--path", str(ROOT), "--root", str(root),
                    "--debug", "--locked", "--offline"], cwd=home, env=env, check=True)
    binary = root / "bin/reedline-ai"
    subprocess.run([str(binary), "adapter", "doctor", "--shell", "powershell"], cwd=home, env=env, check=True)
    assets = home / "PS adapter with spaces"
    subprocess.run([str(binary), "adapter", "install", "--shell", "powershell", "--experimental",
                    "--directory", str(assets)], cwd=home, env=env, check=True)
    module = assets / "ReedlineAI.psm1"
    subprocess.run([str(PWSH), "-NoLogo", "-NoProfile", "-NonInteractive", "-File",
                    str(ROOT / "integrations/powershell/tests/Test-ReedlineAI.ps1"),
                    "-Binary", str(binary), "-ModulePath", str(module)], cwd=home, env=env, check=True)
    env["RAI_POWERSHELL_ADAPTER_PATH"] = str(module)
    subprocess.run([sys.executable, str(ROOT / "integrations/powershell/tests/pty_smoke.py"),
                    str(PWSH), str(binary)], cwd=home, env=env, check=True)
    subprocess.run([str(binary), "adapter", "uninstall", "--shell", "powershell",
                    "--directory", str(assets)], cwd=home, env=env, check=True)
    assert not assets.exists()
    assert not (home / ".config/powershell/Microsoft.PowerShell_profile.ps1").exists()
    print("Installed PowerShell PASS: clean-HOME offline install, doctor, bundled module/helper, 66 assertions, native PTY/disable, uninstall. POSIX host only; Windows terminal validation remains separate.")
