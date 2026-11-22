#!/usr/bin/env bash
# Build a release of termker and package it in dist/.
#
#   ./build.sh            build for this computer (macOS or Linux)
#   ./build.sh --windows  build a Windows .exe with the GNU toolchain
#                         (MinGW-w64), from macOS or Linux
#
# The program is also copied to this folder: termker (Linux),
# termker.app (macOS) or termker.exe (Windows).
#
# On Windows itself, use build.ps1 instead.

set -euo pipefail
cd "$(dirname "$0")"

APP=termker
WINDOWS_TARGET=x86_64-pc-windows-gnu

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
}

fail() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

version() {
    sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1
}

# zip -r <archive> <entries...>, run inside the folder that holds the entries.
make_zip() {
    local archive=$1
    shift
    if command -v zip >/dev/null 2>&1; then
        zip -qr "$archive" "$@"
    elif command -v python3 >/dev/null 2>&1; then
        python3 -m zipfile -c "$archive" "$@"
    else
        fail "need 'zip' or 'python3' to make the .zip file"
    fi
}

need_cargo() {
    command -v cargo >/dev/null 2>&1 ||
        fail "Rust is not installed. Install it from https://rustup.rs and open a new terminal."
}

build_windows() {
    need_cargo
    command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 || fail "the MinGW-w64 compiler is missing.
  Debian/Ubuntu: sudo apt install gcc-mingw-w64-x86-64
  Fedora:        sudo dnf install mingw64-gcc
  macOS:         brew install mingw-w64"
    # windres puts the icon into the .exe. It comes with the same packages.
    command -v x86_64-w64-mingw32-windres >/dev/null 2>&1 ||
        fail "x86_64-w64-mingw32-windres is missing. It is part of the MinGW-w64 packages above."

    if command -v rustup >/dev/null 2>&1; then
        rustup target list --installed | grep -qx "$WINDOWS_TARGET" ||
            rustup target add "$WINDOWS_TARGET"
    fi

    cargo build --release --locked --target "$WINDOWS_TARGET"

    local name="$APP-$(version)-windows-x86_64"
    local stage="dist/$name"
    rm -rf "$stage" "dist/$name.zip"
    mkdir -p "$stage"
    cp "target/$WINDOWS_TARGET/release/$APP.exe" README.md LICENSE-MIT LICENSE-APACHE "$stage/"
    cp "target/$WINDOWS_TARGET/release/$APP.exe" "./$APP.exe"
    (cd dist && make_zip "$name.zip" "$name")
    rm -rf "$stage"
    echo "Built dist/$name.zip and copied $APP.exe here"
}

build_macos() {
    need_cargo
    cargo build --release --locked

    local arch name stage bundle
    arch=$(uname -m)
    name="$APP-$(version)-macos-$arch"
    stage="dist/$name"
    bundle="$stage/$APP.app"
    rm -rf "$stage" "dist/$name.zip"
    mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
    cp "target/release/$APP" "$bundle/Contents/MacOS/$APP"
    cp "assets/icon/$APP.icns" "$bundle/Contents/Resources/$APP.icns"
    cat >"$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP</string>
    <key>CFBundleDisplayName</key><string>$APP</string>
    <key>CFBundleIdentifier</key><string>io.github.profawie.$APP</string>
    <key>CFBundleExecutable</key><string>$APP</string>
    <key>CFBundleIconFile</key><string>$APP</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>$(version)</string>
    <key>CFBundleShortVersionString</key><string>$(version)</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
    cp README.md LICENSE-MIT LICENSE-APACHE "$stage/"
    rm -rf "./$APP.app"
    cp -R "$bundle" "./$APP.app"
    # ditto keeps the bundle's permissions and extended attributes intact.
    (cd dist && ditto -c -k --keepParent "$name" "$name.zip")
    rm -rf "$stage"
    echo "Built dist/$name.zip and copied $APP.app here"
    echo "The app is not signed. On first launch, right-click it and choose Open."
}

build_linux() {
    need_cargo
    cargo build --release --locked

    local name stage
    name="$APP-$(version)-linux-$(uname -m)"
    stage="dist/$name"
    rm -rf "$stage" "dist/$name.tar.gz"
    mkdir -p "$stage"
    cp "target/release/$APP" README.md LICENSE-MIT LICENSE-APACHE "$stage/"
    cp "target/release/$APP" "./$APP"
    tar -C dist -czf "dist/$name.tar.gz" "$name"
    rm -rf "$stage"
    echo "Built dist/$name.tar.gz and copied $APP here"
}

case "${1:-}" in
"")
    case "$(uname -s)" in
    Darwin) build_macos ;;
    Linux) build_linux ;;
    *) fail "unsupported system $(uname -s); on Windows, run build.ps1" ;;
    esac
    ;;
--windows) build_windows ;;
-h | --help) usage ;;
*)
    usage >&2
    exit 2
    ;;
esac
