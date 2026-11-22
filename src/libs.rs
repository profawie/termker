//! Libraries used by termker, for the About screen.
//!
//! Versions come from `Cargo.lock` at build time (see `build.rs`). Purpose,
//! website and license are maintained here. The tests below fail if a
//! library in `[workspace.dependencies]` of the root `Cargo.toml` is
//! not listed. Cryptography crates that come
//! in through `russh` are listed as well.

include!(concat!(env!("OUT_DIR"), "/dependency_versions.rs"));

/// What a library is used for, in the order shown on the About screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Security,
    Interface,
    Foundation,
}

impl Category {
    pub const ALL: [Category; 3] = [Self::Security, Self::Interface, Self::Foundation];

    pub fn title(self) -> &'static str {
        match self {
            Self::Security => "SSH and cryptography",
            Self::Interface => "Interface and terminal",
            Self::Foundation => "Data and system",
        }
    }
}

pub struct LibraryInfo {
    pub name: &'static str,
    pub category: Category,
    pub purpose: &'static str,
    pub website: &'static str,
    pub license: &'static str,
}

pub const LIBRARIES: &[LibraryInfo] = &[
    LibraryInfo {
        name: "eframe",
        category: Category::Interface,
        purpose: "Graphical interface (egui framework)",
        website: "https://github.com/emilk/egui",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "russh",
        category: Category::Security,
        purpose: "SSH protocol: connections, authentication, jump hosts",
        website: "https://github.com/warp-tech/russh",
        license: "Apache-2.0",
    },
    LibraryInfo {
        name: "ssh-key",
        category: Category::Security,
        purpose: "SSH keys and certificates: generate, read, encrypt (used through russh)",
        website: "https://github.com/RustCrypto/SSH",
        license: "Apache-2.0 OR MIT",
    },
    LibraryInfo {
        name: "ring",
        category: Category::Security,
        purpose: "Cryptographic primitives for the SSH transport (used through russh)",
        website: "https://github.com/briansmith/ring",
        license: "Apache-2.0 AND ISC",
    },
    LibraryInfo {
        name: "rand",
        category: Category::Security,
        purpose: "Secure random numbers for key generation",
        website: "https://github.com/rust-random/rand",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "keyring",
        category: Category::Security,
        purpose: "Saved passwords in the OS credential store (Windows Credential Manager, macOS Keychain)",
        website: "https://github.com/open-source-cooperative/keyring-rs",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "russh-sftp",
        category: Category::Security,
        purpose: "SFTP protocol: file listing and transfers",
        website: "https://github.com/AspectUnk/russh-sftp",
        license: "Apache-2.0",
    },
    LibraryInfo {
        name: "rfd",
        category: Category::Interface,
        purpose: "Native open and save file dialogs",
        website: "https://github.com/PolyMeilex/rfd",
        license: "MIT",
    },
    LibraryInfo {
        name: "alacritty_terminal",
        category: Category::Interface,
        purpose: "Terminal emulation (escape sequences, screen, scrollback)",
        website: "https://alacritty.org",
        license: "Apache-2.0",
    },
    LibraryInfo {
        name: "tokio",
        category: Category::Foundation,
        purpose: "Async runtime for network connections",
        website: "https://tokio.rs",
        license: "MIT",
    },
    LibraryInfo {
        name: "serde",
        category: Category::Foundation,
        purpose: "Data serialization",
        website: "https://serde.rs",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "serde_json",
        category: Category::Foundation,
        purpose: "JSON format for saved hosts and settings",
        website: "https://github.com/serde-rs/json",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "directories",
        category: Category::Foundation,
        purpose: "Standard per-user config folder on each OS",
        website: "https://github.com/soc/directories-rs",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "uuid",
        category: Category::Foundation,
        purpose: "Unique identifiers for saved items",
        website: "https://github.com/uuid-rs/uuid",
        license: "MIT OR Apache-2.0",
    },
    LibraryInfo {
        name: "winresource",
        category: Category::Foundation,
        purpose: "Icon and version details of the Windows program (used while building)",
        website: "https://github.com/BenjaminRi/winresource",
        license: "MIT",
    },
];

impl LibraryInfo {
    /// Version built into the app.
    pub fn version(&self) -> &'static str {
        version_of(self.name).unwrap_or("unknown")
    }

    /// Page of exactly this version on crates.io, the registry the library
    /// was downloaded from.
    pub fn registry_url(&self) -> String {
        format!("https://crates.io/crates/{}/{}", self.name, self.version())
    }

    /// The website without `https://`, for display.
    pub fn website_label(&self) -> &'static str {
        self.website
            .trim_start_matches("https://")
            .trim_end_matches('/')
    }
}

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
            assert!(!lib.license.is_empty() && !lib.purpose.is_empty());
        }
    }

    #[test]
    fn every_library_has_a_registry_link_to_its_exact_version() {
        let russh = LIBRARIES.iter().find(|lib| lib.name == "russh").unwrap();
        assert_eq!(
            russh.registry_url(),
            format!("https://crates.io/crates/russh/{}", russh.version())
        );
        assert_eq!(russh.website_label(), "github.com/warp-tech/russh");
        for category in Category::ALL {
            assert!(LIBRARIES.iter().any(|lib| lib.category == category));
        }
    }
}
