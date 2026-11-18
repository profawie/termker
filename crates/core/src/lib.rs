//! The data termker keeps on this computer: hosts, settings, keychain
//! keys and identities, and saved passwords. No networking happens here.

pub mod keychain;
pub mod migrate;
pub mod model;
pub mod os;
pub mod secrets;
pub mod store;
pub mod time;

pub const APP_NAME: &str = "termker";

/// Terminal color theme used until the user picks another one. The themes
/// themselves live in the app, which draws them.
pub const DEFAULT_THEME: &str = "Phosphor";

/// Helpers for tests, here and in the other termker crates.
#[cfg(any(test, feature = "test-support"))]
pub mod testing {
    /// A new, empty folder under the system temp folder.
    pub fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("termker-test-{}-{name}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
