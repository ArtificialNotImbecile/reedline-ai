#!/usr/bin/env python3
"""Install/remove this verified package in a dedicated, explicitly selected root.

No network, elevated privileges, profile edits, PATH changes, or recursive deletion.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import sys

MANIFEST = "manifest.json"
MAX_MANIFEST = 128 * 1024


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def read_manifest(root):
    path = root / MANIFEST
    if path.is_symlink() or not path.is_file() or path.stat().st_size > MAX_MANIFEST:
        raise ValueError("package manifest must be a small ordinary file")
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(manifest, dict) or manifest.get("format") != 1 or not isinstance(manifest.get("files"), dict):
        raise ValueError("unsupported package manifest")
    if not manifest["files"] or len(manifest["files"]) > 100:
        raise ValueError("invalid package file count")
    for name, expected in manifest["files"].items():
        if not isinstance(name, str):
            raise ValueError("invalid package path")
        parts = name.split("/")
        if any(not part or part in (".", "..") or "\\" in part for part in parts):
            raise ValueError("invalid package path")
        if name == MANIFEST or not isinstance(expected, str) or len(expected) != 64:
            raise ValueError("invalid package digest")
        if any(char not in "0123456789abcdef" for char in expected):
            raise ValueError("invalid package digest")
    if "bin/reedline-ai" not in manifest["files"] or "manage.py" not in manifest["files"]:
        raise ValueError("incomplete package manifest")
    return manifest


def expected_entries(manifest):
    files = set(manifest["files"]) | {MANIFEST}
    directories = set()
    for name in files:
        parent = Path(name).parent
        while str(parent) != ".":
            directories.add(parent.as_posix())
            parent = parent.parent
    return files, directories


def validate_tree(root, manifest):
    if root.is_symlink() or not root.is_dir():
        raise ValueError("package root must be an ordinary directory")
    files, directories = expected_entries(manifest)
    actual_files, actual_dirs = set(), set()
    for current, dirs, names in os.walk(root, followlinks=False):
        for name in dirs + names:
            path = Path(current) / name
            relative = path.relative_to(root).as_posix()
            if path.is_symlink():
                raise ValueError("package contains a symlink; nothing removed")
            if path.is_dir():
                actual_dirs.add(relative)
            elif path.is_file():
                actual_files.add(relative)
            else:
                raise ValueError("package contains a non-regular file; nothing removed")
    if files != actual_files or directories != actual_dirs:
        raise ValueError("package has extra or missing files; nothing removed")
    for name, expected in manifest["files"].items():
        if digest(root / name) != expected:
            raise ValueError("package file has changed; nothing removed")


def install(source, destination):
    manifest = read_manifest(source)
    validate_tree(source, manifest)
    if destination.exists() or destination.is_symlink():
        raise ValueError("installation root already exists; choose a new directory")
    if not destination.parent.is_dir():
        raise ValueError("parent directory does not exist; create it explicitly first")
    destination.mkdir(mode=0o700)
    files, directories = expected_entries(manifest)
    # If copying fails, retain the partial directory for inspection; never erase
    # an uncertain destination or retry destructively.
    for directory in sorted(directories, key=lambda value: len(Path(value).parts)):
        (destination / directory).mkdir(mode=0o755)
    for name in sorted(files):
        with (source / name).open("rb") as src, (destination / name).open("xb") as dst:
            shutil.copyfileobj(src, dst)
        (destination / name).chmod(0o755 if name == "bin/reedline-ai" else 0o644)
    validate_tree(destination, read_manifest(destination))
    print("Installed successfully. No shell profiles or PATH were changed.")
    print("Binary:", destination / "bin/reedline-ai")
    print("Quickstart:", destination / "docs/linux-release.md")
    print("Uninstall:", shlex.join(["python3", str(destination / "manage.py"), "uninstall", "--prefix", str(destination)]))


def uninstall(destination):
    manifest = read_manifest(destination)
    validate_tree(destination, manifest)
    files, directories = expected_entries(manifest)
    # Validation above finishes before the first unlink. A concurrent local
    # writer can race this check; stop all adapter sessions first. This is not
    # a security boundary against other processes running as the same user.
    for name in sorted(files):
        (destination / name).unlink()
    for directory in sorted(directories, key=lambda value: len(Path(value).parts), reverse=True):
        (destination / directory).rmdir()
    destination.rmdir()
    print("Removed unchanged package files. Config, history, adapter assets, and shell profiles were not touched.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("install", "uninstall"))
    parser.add_argument("--prefix", required=True, type=Path,
                        help="Dedicated absolute install root, not a shared /usr or ~/.local root")
    args = parser.parse_args()
    destination = args.prefix.expanduser()
    if not destination.is_absolute() or destination == Path(destination.anchor):
        parser.error("--prefix must be a dedicated absolute directory")
    if any(part in (".", "..") for part in args.prefix.parts):
        parser.error("--prefix must not contain dot segments")
    try:
        if args.action == "install":
            install(Path(__file__).resolve().parent, destination)
        else:
            uninstall(destination)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print("reedline-ai package:", error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
