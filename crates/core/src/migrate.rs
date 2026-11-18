//! Moves data saved under the app's earlier name, rterminalfy, to termker.
//!
//! Runs once at start-up. It only acts when there is a folder under the old
//! name and none yet under the new one, so it never overwrites anything.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{APP_NAME, secrets};

/// The name the app had before it was called termker.
pub const OLD_APP_NAME: &str = "rterminalfy";

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// There was nothing under the old name, or termker already has data.
    NothingToMove,
    Moved {
        from: PathBuf,
        to: PathBuf,
        /// Saved passwords moved in the system credential store.
        passwords: usize,
        /// Saved passwords that could not be moved; they stay under the
        /// old name and are asked for again when needed.
        passwords_failed: usize,
    },
}

/// Move the old config folder (hosts, settings, keys, known hosts, the list
/// of saved passwords) and the saved passwords themselves.
pub fn from_old_name() -> io::Result<Outcome> {
    let (Some(old), Some(new)) = (
        directories::ProjectDirs::from("", "", OLD_APP_NAME),
        directories::ProjectDirs::from("", "", APP_NAME),
    ) else {
        return Ok(Outcome::NothingToMove);
    };
    let (from, to) = (old.config_dir(), new.config_dir());
    if !move_folder(from, to)? {
        return Ok(Outcome::NothingToMove);
    }
    let (passwords, passwords_failed) =
        secrets::move_from_service(&to.join("secrets.json"), OLD_APP_NAME);
    Ok(Outcome::Moved {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        passwords,
        passwords_failed,
    })
}

/// Move the folder `from` to `to` if `from` exists and `to` does not.
/// Returns whether it moved.
fn move_folder(from: &Path, to: &Path) -> io::Result<bool> {
    if !from.is_dir() || to.exists() {
        return Ok(false);
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::rename(from, to).is_err() {
        // Renaming fails across drives; copy instead, then remove the old.
        copy_dir(from, to)?;
        fs::remove_dir_all(from)?;
    }
    // On Windows the folder is `<AppData>\rterminalfy\config`; remove the
    // `rterminalfy` folder too if that left it empty. Nothing else is touched.
    if let Some(parent) = from.parent()
        && parent.file_name().is_some_and(|name| name == OLD_APP_NAME)
    {
        let _ = fs::remove_dir(parent);
    }
    Ok(true)
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            // fs::copy keeps the file's permissions, so private keys stay
            // readable only by the user.
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::temp_dir;

    fn old_folder(root: &Path) -> PathBuf {
        let old = root.join(OLD_APP_NAME).join("config");
        fs::create_dir_all(old.join("keys")).unwrap();
        fs::write(old.join("data.json"), "{}").unwrap();
        fs::write(old.join("keys").join("a.key"), "key").unwrap();
        old
    }

    #[test]
    fn old_folder_moves_to_the_new_name() {
        let root = temp_dir("migrate");
        let old = old_folder(&root);
        let new = root.join(APP_NAME).join("config");

        assert!(move_folder(&old, &new).unwrap());
        assert_eq!(fs::read_to_string(new.join("data.json")).unwrap(), "{}");
        assert_eq!(
            fs::read_to_string(new.join("keys").join("a.key")).unwrap(),
            "key"
        );
        assert!(!old.exists());
        // The emptied `rterminalfy` folder is removed as well.
        assert!(!root.join(OLD_APP_NAME).exists());
        // A second start finds nothing to do.
        assert!(!move_folder(&old, &new).unwrap());
    }

    #[test]
    fn existing_new_data_is_never_overwritten() {
        let root = temp_dir("migrate-existing");
        let old = old_folder(&root);
        let new = root.join(APP_NAME).join("config");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("data.json"), "new").unwrap();

        assert!(!move_folder(&old, &new).unwrap());
        assert_eq!(fs::read_to_string(new.join("data.json")).unwrap(), "new");
        assert!(old.join("data.json").exists());
    }

    #[test]
    fn nothing_to_move_without_an_old_folder() {
        let root = temp_dir("migrate-none");
        let new = root.join(APP_NAME);
        assert!(!move_folder(&root.join(OLD_APP_NAME), &new).unwrap());
        assert!(!new.exists());
    }

    #[test]
    fn copy_keeps_every_file() {
        let root = temp_dir("migrate-copy");
        let old = old_folder(&root);
        let copy = root.join("copy");
        copy_dir(&old, &copy).unwrap();
        assert_eq!(
            fs::read_to_string(copy.join("keys").join("a.key")).unwrap(),
            "key"
        );
    }
}
