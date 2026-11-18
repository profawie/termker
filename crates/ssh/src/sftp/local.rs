//! Files on this computer, listed the same way as files on a server so the
//! SFTP page can show both side by side.

use std::path::{Path, PathBuf};

use super::{Entry, EntryKind, sort_entries};

/// Folder a local pane starts in: the user's home folder.
pub fn home() -> PathBuf {
    std::env::home_dir()
        .filter(|path| path.is_dir())
        .unwrap_or_else(|| {
            roots()
                .into_iter()
                .next()
                .unwrap_or_else(|| PathBuf::from("/"))
        })
}

/// Top-level places: the drives on Windows, `/` elsewhere.
pub fn roots() -> Vec<PathBuf> {
    if cfg!(windows) {
        ('A'..='Z')
            .map(|letter| PathBuf::from(format!("{letter}:\\")))
            .filter(|drive| drive.is_dir())
            .collect()
    } else {
        vec![PathBuf::from("/")]
    }
}

/// Everything in `path`, folders first. Links are shown as links.
pub fn list(path: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        // Details of the entry itself; for a link, whether it leads to a
        // folder decides if it can be opened.
        let meta = entry.metadata().ok();
        let kind = match entry.file_type() {
            Ok(kind) if kind.is_symlink() => EntryKind::Symlink,
            Ok(kind) if kind.is_dir() => EntryKind::Dir,
            Ok(kind) if kind.is_file() => EntryKind::File,
            _ => EntryKind::Other,
        };
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().to_string(),
            kind,
            size: meta.as_ref().map(|meta| meta.len()).unwrap_or(0),
            modified: meta.as_ref().and_then(|meta| meta.modified().ok()),
            permissions: meta.as_ref().and_then(permissions),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

#[cfg(unix)]
fn permissions(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn permissions(_: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_first_with_sizes() {
        let dir = crate::tests::temp_dir("local-list");
        std::fs::write(dir.join("b.txt"), b"12345").unwrap();
        std::fs::create_dir(dir.join("Zeta")).unwrap();
        std::fs::write(dir.join("a.txt"), b"1").unwrap();
        let entries = list(&dir).unwrap();
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["Zeta", "a.txt", "b.txt"]);
        assert_eq!(entries[0].kind, EntryKind::Dir);
        assert_eq!(entries[2].size, 5);
        assert!(list(&dir.join("missing")).is_err());
    }

    #[test]
    fn home_and_roots_exist() {
        assert!(home().is_dir());
        assert!(roots().iter().all(|root| root.is_dir()));
    }
}
