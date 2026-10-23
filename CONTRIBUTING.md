# Contributing

Use the pinned Rust toolchain and run `bash scripts/ci-local.sh` on Linux/macOS before proposing changes. It requires Bash and Python 3 and includes formatting, lint, debug/release tests, builds, and terminal smoke checks. For Windows, use the individual Rust commands in README; POSIX PTY checks are not supported there. Include a regression test for fixes.

Validation is local-first. GitHub Actions only runs through an explicit manual `workflow_dispatch`; pushes and pull requests do not trigger hosted CI. Do not start hosted runs or use paid model APIs as part of routine validation.

Keep these boundaries intact:

1. No execution of suggested commands.
2. No implicit network access, file/history harvesting, key persistence, or sensitive logging.
3. UTF-8 byte cursors; legacy FIM inserts preserve suffixes. Structured local edits require full source-buffer/cursor identity and explicit preview acceptance.
4. One bounded worker/queue; never block the editor on inference.
5. No real credentials or paid services in tests.

For terminal behavior changes, manually test typing while a completion is pending, changing the cursor, canceling, closing a menu, and accepting a result after an edit. State which OS/terminal was tested. Do not claim untested providers or terminals work.


Native experimental runtimes have a separate mandatory opt-in gate when changing
those adapters: set `REEDLINE_AI_PWSH` and `NU_BINARY` to installed PowerShell/Nu
executables, then run `bash scripts/ci-experimental-local.sh` after the ordinary
local gate. It refuses missing runtimes instead of reporting a skipped suite as
native validation. It includes self-contained installed-module checks in a clean
HOME. This Linux/POSIX gate is not a Windows CMD/Clink test. Do not change profiles,
execution policies, real histories or credentials to make these tests pass.
