# rterminalfy

A desktop SSH client for individuals, written entirely in Rust. Runs on Windows and macOS.

Version: 0.1.0

## Features

| Feature | Status |
| --- | --- |
| Host management with groups and search | Done |
| Themes (default: green text on black) | Done |
| About screen listing every library, its version and website | Done |
| SSH terminal (password or key file) with host key verification | Done |
| Jump server (ProxyJump), any number of hops | Done |
| Port forwarding: local, remote and dynamic (SOCKS5) | Done |
| SFTP: browse, upload, download, rename, delete, new folder | Done |
| Keychain: generate and import keys (ED25519, ECDSA, RSA), identities, OpenSSH user certificates | Done |
| Saved passwords in the OS keychain | Planned |
| FIDO2 security keys | Planned |

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

Keys in the keychain are stored as one OpenSSH private key file each in a
`keys` folder next to `data.json`, readable only by your user account. A key
is encrypted on disk when you give it a passphrase; without one it is stored
unencrypted, like a key in `~/.ssh`. Public keys and certificates are public
material and are kept in `data.json`.

Trusted host keys are kept in a `known_hosts` file in the same folder, in the
OpenSSH format. Your own `~/.ssh/known_hosts` is never modified.

## Libraries

Every library used is listed on the About screen with its version and
website. Versions are read from `Cargo.lock` at build time, and a test fails
if a library in `Cargo.toml` is not listed. The cryptography libraries that
come in through `russh` (`ssh-key` and `ring`) are listed too.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
