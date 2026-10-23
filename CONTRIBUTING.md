# Contributing

Use the pinned Rust toolchain and run `bash scripts/ci-local.sh` on Linux/macOS before proposing changes. It requires Bash and Python 3 and includes formatting, lint, debug/release tests, builds, and terminal smoke checks. For Windows, use the individual Rust commands in README; POSIX PTY checks are not supported there. Include a regression test for fixes.

Validation is local-first. GitHub Actions only runs through an explicit manual `workflow_dispatch`; pushes and pull requests do not trigger hosted CI. Do not start hosted runs or use paid model APIs as part of routine validation.

Keep these boundaries intact:

1. No execution of suggested commands.
2. No implicit network access, file/history harvesting, key persistence, or sensitive logging.
3. UTF-8 byte cursors; FIM inserts without replacing existing suffixes.
4. One bounded worker/queue; never block the editor on inference.
5. No real credentials or paid services in tests.

For terminal behavior changes, manually test typing while a completion is pending, changing the cursor, canceling, closing a menu, and accepting a result after an edit. State which OS/terminal was tested. Do not claim untested providers or terminals work.
