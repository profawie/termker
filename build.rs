//! Generates the library version list straight from `Cargo.lock`, so the
//! About screen always shows the versions actually built into the app.
//! For Windows it also puts the icon and version details into the `.exe`.

use std::{collections::BTreeMap, env, fs, path::Path};

/// Cryptography crates that come in through `russh` rather than being direct
/// dependencies. They do the security-critical work, so About lists them too.
const BUNDLED_CRYPTO: &[&str] = &["ssh-key", "ring"];

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");

    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml");
    let lock = fs::read_to_string("Cargo.lock").expect("Cargo.lock");

    let mut direct = direct_dependencies(&manifest);
    direct.extend(BUNDLED_CRYPTO.iter().map(|name| name.to_string()));
    let locked = locked_versions(&lock);

    let mut out = String::from("pub const DEPENDENCY_VERSIONS: &[(&str, &str)] = &[\n");
    for name in &direct {
        let version = locked
            .get(name)
            .unwrap_or_else(|| panic!("{name} is missing from Cargo.lock"));
        out.push_str(&format!("    ({name:?}, {version:?}),\n"));
    }
    out.push_str("];\n");

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("dependency_versions.rs");
    fs::write(dest, out).unwrap();

    windows_resources();
}

/// Icon and file details (name, version, description) of the Windows
/// program, as shown by Explorer and the taskbar. Needs `windres`, which
/// comes with the MinGW-w64 compiler the Windows build already uses.
fn windows_resources() {
    println!("cargo:rerun-if-changed=assets/icon/termker.ico");
    // The build script runs on the building computer; ask what it builds for.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resources = winresource::WindowsResource::new();
    resources
        .set_icon("assets/icon/termker.ico")
        .set("ProductName", "termker")
        .set("FileDescription", "termker")
        .set("LegalCopyright", "MIT OR Apache-2.0");
    resources
        .compile()
        .expect("windres (part of MinGW-w64) embeds the Windows icon");
}

/// Name of every third-party library. They are all declared once, in the
/// workspace's `[workspace.dependencies]`; the app's own crates
/// (`path = ...`) are left out.
fn direct_dependencies(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line == "[workspace.dependencies]";
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') || line.contains("path =") {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().to_string());
        }
    }
    names
}

/// Highest version of each package in `Cargo.lock`.
fn locked_versions(lock: &str) -> BTreeMap<String, String> {
    let mut versions: BTreeMap<String, String> = BTreeMap::new();
    let mut name: Option<String> = None;
    for line in lock.lines() {
        if let Some(value) = line.strip_prefix("name = ") {
            name = Some(value.trim_matches('"').to_string());
        } else if let Some(value) = line.strip_prefix("version = ")
            && let Some(name) = name.take()
        {
            let version = value.trim_matches('"').to_string();
            let newer = versions
                .get(&name)
                .is_none_or(|old| version_key(&version) > version_key(old));
            if newer {
                versions.insert(name, version);
            }
        }
    }
    versions
}

fn version_key(version: &str) -> Vec<u64> {
    version
        .split(['.', '-', '+'])
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}
