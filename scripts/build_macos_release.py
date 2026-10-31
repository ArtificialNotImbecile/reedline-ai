#!/usr/bin/env python3
"""Package a tested native macOS binary and its Bash trial launcher locally."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import platform
import re
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]


def output(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def build(binary, destination):
    if platform.system() != "Darwin":
        raise ValueError("native macOS host required")
    binary = binary.resolve(strict=True)
    version = output(str(binary), "--version").removeprefix("reedline-ai ")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("unexpected binary version")
    architecture = platform.machine()
    if architecture not in ("arm64", "x86_64"):
        raise ValueError("unsupported native architecture")
    if output("lipo", "-archs", str(binary)) != architecture:
        raise ValueError("binary architecture must match host")
    minimum = re.search(r"\bminos ([0-9.]+)", output("xcrun", "vtool", "-show-build", str(binary)))
    if not minimum:
        raise ValueError("cannot determine deployment minimum")
    files = {"bin/reedline-ai": binary.read_bytes()}
    for name in output("git", "ls-files", "--", "docs", "integrations/bash",
                       "scripts/try-bash.sh", "README.md", "LICENSE", "THIRD_PARTY_NOTICES.md").splitlines():
        files[name] = (ROOT / name).read_bytes()
    manifest = {
        "format": 1, "version": version, "os": "macos", "architecture": architecture,
        "minimum_macos": minimum[1], "validation_host": platform.mac_ver()[0],
        "source_revision": output("git", "rev-parse", "HEAD"),
        "source_tracked_dirty": bool(output("git", "status", "--porcelain", "--untracked-files=no")),
        "files": {name: hashlib.sha256(data).hexdigest() for name, data in sorted(files.items())},
        "note": "Native build; not universal or notarized. Deployment minimum is not validation on every macOS version.",
    }
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    destination.mkdir(parents=True, exist_ok=True)
    name = f"reedline-ai-{version}-macos-{architecture}"
    archive = destination / (name + ".tar.gz")
    with archive.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as tar:
            for path, data in sorted(files.items()):
                entry = tarfile.TarInfo(f"{name}/{path}")
                entry.size = len(data)
                entry.mode = 0o755 if path == "bin/reedline-ai" else 0o644
                tar.addfile(entry, io.BytesIO(data))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (destination / (archive.name + ".sha256")).write_text(f"{digest}  {archive.name}\n")
    print(json.dumps({"archive": str(archive.resolve()), "sha256": digest, **manifest}))
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/reedline-ai")
    parser.add_argument("--output", type=Path, default=ROOT / "target/dist")
    args = parser.parse_args()
    build(args.binary, args.output)
