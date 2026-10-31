#!/usr/bin/env python3
"""Create a deterministic Linux prebuilt archive from an already-built binary.

Does not build, download, publish, call a model, or trigger hosted CI. Build and
test the selected binary first. Artifact identity includes its exact SHA256.
"""
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
    if platform.system() != "Linux":
        raise ValueError("this packaging script supports native Linux builds only")
    binary = binary.resolve(strict=True)
    version_output = output(str(binary), "--version")
    match = re.fullmatch(r"reedline-ai ([0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?)", version_output)
    if not match:
        raise ValueError("selected binary did not report a reedline-ai version")
    architecture = platform.machine()
    if architecture not in ("x86_64", "aarch64"):
        raise ValueError("unsupported native architecture")
    elf_header = output("readelf", "-h", str(binary))
    expected_machine = {"x86_64": "Advanced Micro Devices X86-64", "aarch64": "AArch64"}[architecture]
    if expected_machine not in elf_header:
        raise ValueError("binary architecture does not match packaging host")
    version_info = output("readelf", "--version-info", str(binary))
    # Weak symbol versions are optional runtime fallbacks, not loader minimums.
    versions = re.findall(r"Name: GLIBC_(\d+)\.(\d+)(?:\.(\d+))?\s+Flags: none", version_info)
    minimum = max((tuple(int(part or "0") for part in version) for version in versions), default=None)
    if minimum is None:
        raise ValueError("cannot determine glibc minimum from binary")
    minimum_text = ".".join(map(str, minimum[:2] if minimum[2] == 0 else minimum))
    needed = re.findall(r"\(NEEDED\).*\[([^\]]+)\]", output("readelf", "--dynamic", str(binary)))
    revision = output("git", "rev-parse", "HEAD")
    dirty = bool(output("git", "status", "--porcelain", "--untracked-files=no"))
    files = {
        "bin/reedline-ai": binary.read_bytes(),
        "manage.py": (ROOT / "packaging/linux/manage.py").read_bytes(),
        "LICENSE": (ROOT / "LICENSE").read_bytes(),
        "THIRD_PARTY_NOTICES.md": (ROOT / "THIRD_PARTY_NOTICES.md").read_bytes(),
        "README.md": (ROOT / "README.md").read_bytes(),
    }
    # Ship the tracked documentation set so the setup guide and its local
    # adapter/privacy links work without keeping the source checkout.
    for name in output("git", "ls-files", "--", "docs").splitlines():
        if name.startswith("docs/") and name.endswith(".md"):
            files[name] = (ROOT / name).read_bytes()
    manifest = {
        "format": 1, "version": match[1], "os": "linux", "architecture": architecture,
        "libc": "glibc", "minimum_glibc": minimum_text,
        "needed_shared_libraries": needed,
        "source_revision": revision, "source_tracked_dirty": dirty,
        "binary_sha256": hashlib.sha256(files["bin/reedline-ai"]).hexdigest(),
        "files": {name: hashlib.sha256(contents).hexdigest() for name, contents in sorted(files.items())},
        "note": "Host-native build, not a universal Linux or musl package. No claim that its source revision proves binary provenance; build/test selected binary before packaging.",
    }
    files["manifest.json"] = (json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()
    name = f"reedline-ai-{match[1]}-linux-{architecture}-glibc"
    destination.mkdir(parents=True, exist_ok=True)
    archive = destination / f"{name}.tar.gz"
    # Tar and gzip metadata are normalized; equal inputs produce equal bytes.
    with archive.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as tar:
            for path, contents in sorted(files.items()):
                entry = tarfile.TarInfo(f"{name}/{path}")
                entry.size = len(contents)
                entry.mode = 0o755 if path == "bin/reedline-ai" else 0o644
                entry.uid = entry.gid = entry.mtime = 0
                entry.uname = entry.gname = ""
                tar.addfile(entry, io.BytesIO(contents))
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    (destination / f"{archive.name}.sha256").write_text(f"{checksum}  {archive.name}\n", encoding="utf-8")
    print(json.dumps({"archive": str(archive.resolve()), "sha256": checksum,
                      "minimum_glibc": minimum_text, "source_revision": revision,
                      "source_tracked_dirty": dirty}, sort_keys=True))
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/reedline-ai")
    parser.add_argument("--output", type=Path, default=ROOT / "target/dist")
    args = parser.parse_args()
    build(args.binary, args.output)


if __name__ == "__main__":
    main()
