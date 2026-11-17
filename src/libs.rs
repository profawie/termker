//! Senarai library yang digunakan oleh rterminalfy, untuk skrin About.
//!
//! Versi datang daripada `Cargo.lock` semasa binaan (lihat `build.rs`).
//! Tujuan, laman web dan lesen diselenggara di sini. Ujian di bawah gagal
//! jika ada kebergantungan dalam `Cargo.toml` yang belum disenaraikan.

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

/// Versi yang dibina ke dalam aplikasi bagi library bernama `name`.
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
                "{dep} ada dalam Cargo.toml tetapi tiada dalam LIBRARIES"
            );
        }
    }

    #[test]
    fn every_listed_library_is_a_real_dependency() {
        for lib in LIBRARIES {
            assert!(
                version_of(lib.name).is_some(),
                "{} disenaraikan tetapi bukan kebergantungan",
                lib.name
            );
            assert!(lib.website.starts_with("https://"));
        }
    }
}
