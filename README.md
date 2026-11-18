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
| Saved passwords and passphrases in the OS credential store (opt-in) | Done |
| FIDO2 security keys (`ed25519-sk`, `ecdsa-sk`): create, import, sign in | Done |

## Build and run

Requires [Rust](https://rustup.rs) 1.92 or newer.

```
cargo run --release
```

### Release packages

The build scripts make a release build and package it in `dist/`.

**Windows**, with the GNU toolchain, so no Visual Studio or Microsoft build
tools are needed:

1. Install rustup from [rustup.rs](https://rustup.rs). If it is not installed
   yet, run the installer as `rustup-init.exe --default-host x86_64-pc-windows-gnu -y`.
2. Install a 64-bit MinGW-w64 gcc (MSVCRT build), for example
   `winget install -e --id BrechtSanders.WinLibs.POSIX.MSVCRT`, or MSYS2 with
   `pacman -S mingw-w64-x86_64-gcc`. Open a new terminal afterwards.
3. Run:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\build.ps1
   ```

The script installs Rust's `stable-x86_64-pc-windows-gnu` toolchain if it is
missing and writes `dist\rterminalfy-<version>-windows-x86_64.zip`. The `.exe`
inside needs no other files.

**macOS and Linux:**

```
./build.sh             # .app in a .zip on macOS, .tar.gz on Linux
./build.sh --windows   # Windows .zip, cross-built with MinGW-w64
```

The macOS app is not signed. On first launch, right-click it and choose Open.

## Tests

```
cargo test
```

## Data

Hosts and settings are stored as JSON in the per-user config folder
(`%APPDATA%\rterminalfy\config\data.json` on Windows,
`~/Library/Application Support/rterminalfy/data.json` on macOS).
Passwords and private keys are never written to this file.

Passwords and key passphrases are asked for when connecting. If you tick
"Remember", they are saved in the operating system's credential store
(Windows Credential Manager or the macOS Keychain), never in a file of this
app. A `secrets.json` file lists the names of what was saved, without the
secrets, so the Keychain page can show the list and remove entries.

Keys in the keychain are stored as one OpenSSH private key file each in a
`keys` folder next to `data.json`, readable only by your user account. A key
is encrypted on disk when you give it a passphrase; without one it is stored
unencrypted, like a key in `~/.ssh`. Public keys and certificates are public
material and are kept in `data.json`.

A key on a FIDO2 security key never leaves it. The file kept for it holds
only a key handle, in the same format `ssh-keygen -t ed25519-sk` writes, and
is useless without the security key. Its PIN is never saved.

Security keys are reached over USB directly on macOS, and through Windows' own
WebAuthn service on Windows, which shows the "Windows Security" window for the
PIN and the touch. This needs Windows 10 version 1903 or newer.

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
