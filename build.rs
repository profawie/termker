//! Menjana senarai versi library terus daripada `Cargo.lock`, supaya skrin
//! About sentiasa memaparkan versi yang benar-benar dibina ke dalam aplikasi.

use std::{collections::BTreeMap, env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");

    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml");
    let lock = fs::read_to_string("Cargo.lock").expect("Cargo.lock");

    let direct = direct_dependencies(&manifest);
    let locked = locked_versions(&lock);

    let mut out = String::from("pub const DEPENDENCY_VERSIONS: &[(&str, &str)] = &[\n");
    for name in &direct {
        let version = locked
            .get(name)
            .unwrap_or_else(|| panic!("{name} tiada dalam Cargo.lock"));
        out.push_str(&format!("    ({name:?}, {version:?}),\n"));
    }
    out.push_str("];\n");

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("dependency_versions.rs");
    fs::write(dest, out).unwrap();
}

/// Nama setiap kebergantungan dalam bahagian `[dependencies]`.
fn direct_dependencies(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line == "[dependencies]";
            continue;
        }
        if !in_deps || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().to_string());
        }
    }
    names
}

/// Versi tertinggi bagi setiap pakej dalam `Cargo.lock`.
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
