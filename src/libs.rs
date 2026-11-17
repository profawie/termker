//! Libraries used by rterminalfy, for the About screen.
//!
//! Versions come from `Cargo.lock` at build time (see `build.rs`). Purpose,
//! website and license are maintained here. The tests below fail if a
//! dependency in `Cargo.toml` is not listed.

include!(concat!(env!("OUT_DIR"), "/dependency_versions.rs"));

pub struct LibraryInfo {
    pub name: &'static str,
    pub purpose: &'static str,
    pub website: &'static str,
    pub license: &'static str,
}

pub const LIBRARIES: &[LibraryInfo] = &[
    LibraryInfo {
        name: "eframe",
        purpose: "Graphical interface (egui framework)",
        website: "https://github.com/emilk/egui",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "serde",
        purpose: "Data serialization",
        website: "https://serde.rs",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "serde_json",
        purpose: "JSON format for saved hosts and settings",
        website: "https://github.com/serde-rs/json",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "directories",
        purpose: "Standard per-user config folder on each OS",
        website: "https://github.com/soc/directories-rs",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "uuid",
        purpose: "Unique identifiers for saved items",
        website: "https://github.com/uuid-rs/uuid",
        license: "MIT OR Apache-2.0",
    },
];

/// Version of the library `name` that is built into the app.
pub fn version_of(name: &str) -> Option<&'static str> {
    DEPENDENCY_VERSIONS
        .iter()
        .find(|(dep, _)| *dep == name)
        .map(|(_, version)| *version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dependency_is_listed_in_about() {
        for (dep, _) in DEPENDENCY_VERSIONS {
            assert!(
                LIBRARIES.iter().any(|lib| lib.name == *dep),
                "{dep} is in Cargo.toml but missing from LIBRARIES"
            );
        }
    }

    #[test]
    fn every_listed_library_is_a_real_dependency() {
        for lib in LIBRARIES {
            assert!(
                version_of(lib.name).is_some(),
                "{} is listed but is not a dependency",
                lib.name
            );
            assert!(lib.website.starts_with("https://"));
        }
    }
}
