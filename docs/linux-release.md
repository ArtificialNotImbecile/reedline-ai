# Linux prebuilt package

This archive includes a compiled `reedline-ai`; Rust/Cargo is **not required** to
install or run it. It is a **native Linux glibc build**, not a universal binary.
Check `manifest.json` for the exact CPU architecture, minimum glibc version,
required shared libraries, binary SHA256 and source revision. Alpine/musl, older glibc, macOS and Windows
are not supported by this archive. The release builder checks the ELF's
non-weak symbol versions rather than assuming compatibility from its build host.
This is a loader-requirement estimate, not a test on every qualifying Linux
distribution; the supplied build has only been run on the validation host.

Python 3.9+ is required for the installer and Bash adapter; Bash 4.3+ and a UTF-8
locale are required for that adapter. The standalone binary itself does not
need Python. Existing shell packages are never installed automatically.

## Install without compiling

Download the archive and its `.sha256` sidecar from the same trusted release.
Verify the download, extract it, then enter the extracted directory:

```sh
sha256sum -c reedline-ai-*-linux-*-glibc.tar.gz.sha256
tar -xzf reedline-ai-*-linux-*-glibc.tar.gz
cd reedline-ai-*-linux-*-glibc
python3 manage.py install --prefix "$HOME/reedline-ai"
"$HOME/reedline-ai/bin/reedline-ai" --complete 'git st'
```

Use the exact downloaded filename if multiple archives are present. The prefix
must be a **new dedicated directory** with an existing parent. Existing files
are never overwritten. No `sudo`, profile edit, automatic PATH change, network
request or API call occurs. The install validates every packaged file's hash;
the sidecar provides integrity, not an independent publisher signature.

The smoke command uses deterministic offline examples and prints `git status`;
it does not execute it. The full [setup guide](install.md) is included. The
installer does not add the binary to PATH; use its quoted absolute path:

```sh
BIN="$HOME/reedline-ai/bin/reedline-ai"
"$BIN" setup
eval "$("$BIN" init --shell bash)"
```

Setup asks for Base URL, model name and an optional hidden API key, using only
the OpenAI-compatible chat-completions protocol. It makes no API request. The
key is saved **unencrypted locally only after explicit save confirmation**;
new Unix app directories/files use 0700/0600 permissions, not encryption. Do
not share, sync or commit the settings. Leave the key blank and use your own
secure `REEDLINE_AI_API_KEY` environment mechanism if you do not want it stored.
The environment key takes precedence over a saved key.

The `init` command extracts the bundled adapter and enables it for this Bash
session only, using settings saved by setup. It never edits `.bashrc` or another
profile. You can inspect its generated code by omitting `eval`. Requests occur
only when you press the adapter's request key, and send the current command to
the configured service; do not type secrets. Native history is not uploaded.

For an offline first test, or to avoid network requests despite saved settings,
use this instead (disable an already loaded adapter first):

```sh
BIN="$HOME/reedline-ai/bin/reedline-ai"
"$BIN" adapter doctor --shell bash
eval "$("$BIN" init --shell bash --mock)"
```

In mock mode, type `echo 世` without pressing Enter. Ctrl-x Ctrl-g requests a suggestion;
press Ctrl-x Ctrl-g again to preview it, and once more to accept `界`. Native Tab and
history remain the shell's. Acceptance does not press Enter; pressing Enter
yourself still executes the command normally in Bash. This is explicit
request/preview/accept, not automatic inline AI hints. External model output
is not guaranteed to match mock examples. The included setup and adapter guides
cover risk confirmation and experimental shells.

## Disable and uninstall

In every Bash session using the adapter, disable it before removing the program:

```sh
reedline_ai_disable
python3 "$HOME/reedline-ai/manage.py" uninstall --prefix "$HOME/reedline-ai"
```

The package uninstaller checks all hashes and refuses if files were modified,
removed or added. It never recursively deletes an arbitrary directory. Keep
configuration and personal files outside the installation root. Settings,
saved credentials and the versioned adapter assets created by `init` deliberately
survive package uninstall. See [remove settings and init assets](install.md#remove-settings-and-init-assets)
for explicit optional cleanup; do it with the matching binary **before** removing
that binary. History and shell profiles are not deleted. Any PATH
or profile lines you added manually must be removed manually. Stop active
adapter sessions first; this is not protection against concurrent local file
replacement by another process running as you.

For an upgrade, install the new archive to another dedicated directory, test
it, disable the old adapter and switch the explicit binary/asset paths. There
is deliberately no destructive in-place upgrade.

## Build and verify a release locally

Maintainers build/test first, then run:

```sh
cargo build --locked --release
python3 scripts/build_linux_release.py
python3 scripts/test_linux_release.py
```

The archive builder uses only local files and `readelf`; it does not invoke
Cargo, download dependencies, call a model, publish a release or start GitHub
Actions. Normalized tar/gzip metadata makes identical inputs reproducible.
`source_tracked_dirty` identifies a package built while tracked source edits
exist; distribute an intentionally reviewed commit and record its tests.
Neither a recorded Git revision nor a SHA256 alone proves compiler provenance.
