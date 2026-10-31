# macOS prebuilt preview

The v0.2.0 archive contains a native Apple Silicon executable, documentation and
the Bash trial launcher. Rust is not needed. It is not a universal or notarized
app; the manifest records the build's deployment minimum and actual validation
host. A deployment minimum alone is not a compatibility test.

Download the `.tar.gz` and `.sha256` from the same GitHub release, then verify and
extract them in a directory you control:

```sh
shasum -a 256 -c reedline-ai-0.2.0-macos-arm64.tar.gz.sha256
tar -xzf reedline-ai-0.2.0-macos-arm64.tar.gz
cd reedline-ai-0.2.0-macos-arm64
./bin/reedline-ai --complete 'git st'
bash scripts/try-bash.sh
```

The trial requires Bash 4.3+ and Python 3.9+. macOS's bundled Bash 3.2 cannot run
the adapter; install a newer Bash yourself (for example `brew install bash`).
The launcher finds Homebrew's Bash automatically. It reads the selected Bash
history locally and never writes back or changes profiles. Repeat Ctrl-x Ctrl-g
to request, preview and insert. Ctrl-x Ctrl-k cancels without losing your input.
Exit returns to your original shell.

For the explicitly authorized DeepSeek path, supply `DEEPSEEK_API_KEY` in your
environment and run `bash scripts/try-bash.sh --deepseek`; it uses
`deepseek-flash`. Only the invoked current command is sent; history matches stay
local. For other endpoints, use `./bin/reedline-ai setup` and follow the
[setup/init guide](install.md). Nothing sends Enter on your behalf.

Keep the extracted folder wherever you want and use the quoted executable path.
There is no automatic PATH change or profile install. To remove it, disable any
active adapter, exit the trial and remove this extracted folder. Settings and
versioned assets created through setup/init live separately; see the setup guide
for their explicit cleanup.

Maintainers build and test the release binary, then run
`python3 scripts/build_macos_release.py` from the reviewed commit. The packager
records file hashes, source revision and whether tracked files were dirty; it
does not build, publish, download or invoke a provider.
