# rterminalfy

A desktop SSH client for individuals, written entirely in Rust. Runs on Windows and macOS.

Version: 0.1.0

## Features

| Feature | Status |
| --- | --- |
| Host management with groups and search | Done |
| Themes (default: green text on black) | Done |
| About screen listing every library, its version and website | Done |
| SSH terminal and jump server | In progress |
| SFTP and port forwarding | In progress |
| Keychain: key generation, identities, certificates, FIDO2 | In progress |

## Build and run

Requires [Rust](https://rustup.rs) 1.92 or newer.

```
cargo run --release
```

## Tests

```
cargo test
```

## Data

Hosts and settings are stored as JSON in the per-user config folder
(`%APPDATA%\rterminalfy\config\data.json` on Windows,
`~/Library/Application Support/rterminalfy/data.json` on macOS).
Passwords and private keys are never written to this file.

## Libraries

Every library used is listed on the About screen with its version and
website. Versions are read from `Cargo.lock` at build time, and a test fails
if a library in `Cargo.toml` is not listed.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
