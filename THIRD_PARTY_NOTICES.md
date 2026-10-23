# Third-party notices

This repository contains original integration code and depends on upstream crates via Cargo. It does not vendor, fork, or relicense Reedline.

Direct dependencies:

- [Reedline](https://github.com/nushell/reedline), MIT. Copyright belongs to the Nushell/Reedline contributors. Core line editing, hints, completion menus, and terminal rendering come from this library.
- [clap](https://github.com/clap-rs/clap), MIT OR Apache-2.0.
- [nu-ansi-term](https://github.com/nushell/nu-ansi-term), MIT.
- [reqwest](https://github.com/seanmonstar/reqwest), MIT OR Apache-2.0.
- [serde](https://github.com/serde-rs/serde), MIT OR Apache-2.0.
- [serde_json](https://github.com/serde-rs/json), MIT OR Apache-2.0.

The complete resolved dependency graph, including transitive and platform-specific dependencies, is recorded in `Cargo.lock`. `cargo metadata --locked --format-version 1` reports their license expressions and source repositories. Upstream license texts are included in the corresponding crates distributed by crates.io; retain applicable notices if redistributing source or compiled binaries. This summary is not a complete binary-distribution license bundle.
