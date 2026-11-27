//! Text typed into a remote shell on the user's behalf.

/// The command that takes a shell on the host to `path`, a folder as SFTP
/// names it, followed by Enter.
///
/// Unix paths are put in single quotes, so spaces, `$` and the like are
/// taken as they are. SFTP on Windows names folders like `/C:/Users/me`;
/// those become `cd "C:\Users\me"`, which cmd and PowerShell both take.
pub fn cd_command(path: &str) -> String {
    if let Some(windows) = windows_path(path) {
        return format!("cd \"{windows}\"\r");
    }
    format!("cd '{}'\r", path.replace('\'', r"'\''"))
}

/// `C:\Users\me` for an SFTP path like `/C:/Users/me` or `C:/Users/me`.
fn windows_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix('/').unwrap_or(path);
    let bytes = rest.as_bytes();
    let drive = bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes.len() == 2 || bytes[2] == b'/' || bytes[2] == b'\\');
    if !drive {
        return None;
    }
    let mut windows = rest.replace('/', "\\");
    if windows.len() == 2 {
        windows.push('\\');
    }
    Some(windows)
}

#[cfg(test)]
mod tests {
    use super::cd_command;

    #[test]
    fn unix_paths_are_quoted() {
        assert_eq!(cd_command("/home/ubuntu"), "cd '/home/ubuntu'\r");
        assert_eq!(cd_command("/srv/my files"), "cd '/srv/my files'\r");
        assert_eq!(cd_command("/tmp/$HOME;rm"), "cd '/tmp/$HOME;rm'\r");
        assert_eq!(
            cd_command("/tmp/it's"),
            r"cd '/tmp/it'\''s'".to_string() + "\r"
        );
    }

    #[test]
    fn windows_drives_use_backslashes() {
        assert_eq!(cd_command("/C:/Users/me"), "cd \"C:\\Users\\me\"\r");
        assert_eq!(cd_command("/D:"), "cd \"D:\\\"\r");
        assert_eq!(cd_command("C:/"), "cd \"C:\\\"\r");
    }
}
