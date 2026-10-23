# Contributing

Use the pinned Rust toolchain and run `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked` before proposing changes. Include a regression test for fixes.

Keep these boundaries intact:

1. No execution of suggested commands.
2. No implicit network access, file/history harvesting, key persistence, or sensitive logging.
3. UTF-8 byte cursors; FIM inserts without replacing existing suffixes.
4. One bounded worker/queue; never block the editor on inference.
5. No real credentials or paid services in tests.

For terminal behavior changes, manually test typing while a completion is pending, changing the cursor, canceling, closing a menu, and accepting a result after an edit. State which OS/terminal was tested. Do not claim untested providers or terminals work.
