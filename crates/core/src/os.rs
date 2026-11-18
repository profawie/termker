//! Which operating system a host runs, as the host itself reports it.

use serde::{Deserialize, Serialize};

const MAX_FIELD_CHARS: usize = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OsFamily {
    Linux,
    MacOs,
    /// BSD, Solaris, AIX and every other Unix.
    Unix,
}

impl OsFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::MacOs => "Mac",
            Self::Unix => "Unix",
        }
    }
}

/// What the remote host said about itself. All text comes from the server
/// and is trimmed to short, printable strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteOs {
    pub family: OsFamily,
    /// Lowercase identifier used to pick the logo: `ubuntu`, `debian`,
    /// `macos`, `freebsd`, ... or `linux` when the distribution is unknown.
    pub id: String,
    /// Identifiers of related systems (`ID_LIKE`), e.g. `debian` for Ubuntu.
    #[serde(default)]
    pub id_like: Vec<String>,
    /// Name to show, e.g. `Ubuntu`, `macOS`, `FreeBSD`.
    pub name: String,
    /// Version of the system, e.g. `24.04`. Empty if it has none.
    #[serde(default)]
    pub version: String,
    /// Kernel name and release as `uname -sr` prints them.
    #[serde(default)]
    pub kernel: String,
}

impl RemoteOs {
    /// Name and version together, e.g. `Ubuntu 24.04`.
    pub fn title(&self) -> String {
        if self.version.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.version)
        }
    }

    /// The family with the kernel version where that adds something, e.g.
    /// `Linux 6.8` under Ubuntu, but only `Unix` under FreeBSD.
    pub fn family_text(&self) -> String {
        let release = self.kernel.split_whitespace().nth(1).unwrap_or_default();
        if self.family != OsFamily::Linux || release.is_empty() || release == self.version {
            return self.family.label().to_string();
        }
        // `6.8.0-45-generic` -> `6.8`.
        let mut short = String::new();
        for (index, part) in release.split('.').take(2).enumerate() {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                break;
            }
            if index > 0 {
                short.push('.');
            }
            short.push_str(&digits);
        }
        if short.is_empty() {
            self.family.label().to_string()
        } else {
            format!("{} {short}", self.family.label())
        }
    }
}

/// Read the answer to the question termker asks a host after signing in:
/// a `TERMKER_UNAME=` line with `uname -sr`, then `os-release` and `sw_vers`
/// where they exist.
pub fn parse(output: &str) -> Option<RemoteOs> {
    let mut uname = String::new();
    let mut release = Fields::default();
    let mut mac_name = String::new();
    let mut mac_version = String::new();

    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("TERMKER_UNAME=") {
            uname = clean(value);
        } else if let Some(value) = line.strip_prefix("ProductName:") {
            mac_name = clean(value);
        } else if let Some(value) = line.strip_prefix("ProductVersion:") {
            mac_version = clean(value);
        } else if let Some((key, value)) = line.split_once('=') {
            let value = clean(&unquote(value));
            match key {
                "ID" => release.id = value.to_lowercase(),
                "ID_LIKE" => {
                    release.id_like = value.split_whitespace().map(str::to_lowercase).collect();
                }
                "NAME" => release.name = value,
                "VERSION_ID" => release.version = value,
                _ => {}
            }
        }
    }

    let kernel_name = uname.split_whitespace().next().unwrap_or_default();
    let kernel_release = uname.split_whitespace().nth(1).unwrap_or_default();

    if kernel_name == "Darwin" || !mac_name.is_empty() {
        return Some(RemoteOs {
            family: OsFamily::MacOs,
            id: "macos".to_string(),
            id_like: Vec::new(),
            name: if mac_name.is_empty() {
                "macOS".to_string()
            } else {
                mac_name
            },
            version: mac_version,
            kernel: uname,
        });
    }
    if kernel_name.is_empty() {
        // Not a Unix answer at all.
        return None;
    }

    let family = if kernel_name == "Linux" {
        OsFamily::Linux
    } else {
        OsFamily::Unix
    };
    let id = if release.id.is_empty() {
        kernel_name.to_lowercase()
    } else {
        release.id
    };
    let (name, version) = if release.name.is_empty() {
        (kernel_name.to_string(), kernel_release.to_string())
    } else {
        // "Debian GNU/Linux" is just Debian.
        let name = release
            .name
            .strip_suffix(" GNU/Linux")
            .unwrap_or(&release.name);
        (name.to_string(), release.version)
    };
    Some(RemoteOs {
        family,
        id,
        id_like: release.id_like,
        name,
        version,
        kernel: uname,
    })
}

#[derive(Default)]
struct Fields {
    id: String,
    id_like: Vec<String>,
    name: String,
    version: String,
}

/// Values in `os-release` may be wrapped in single or double quotes.
fn unquote(value: &str) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.replace('\\', "");
        }
    }
    value.to_string()
}

/// Keep what the server sent short and printable; it ends up on screen.
fn clean(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_FIELD_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ubuntu_is_named_with_its_version_and_kernel() {
        let os = parse(
            "TERMKER_UNAME=Linux 6.8.0-45-generic\n\
             PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n\
             NAME=\"Ubuntu\"\n\
             VERSION_ID=\"24.04\"\n\
             VERSION=\"24.04.1 LTS (Noble Numbat)\"\n\
             ID=ubuntu\n\
             ID_LIKE=debian\n",
        )
        .unwrap();
        assert_eq!(os.family, OsFamily::Linux);
        assert_eq!(os.id, "ubuntu");
        assert_eq!(os.id_like, ["debian"]);
        assert_eq!(os.title(), "Ubuntu 24.04");
        assert_eq!(os.family_text(), "Linux 6.8");
        assert_eq!(os.kernel, "Linux 6.8.0-45-generic");
    }

    #[test]
    fn debian_drops_gnu_linux_and_rolling_releases_have_no_version() {
        let debian = parse(
            "TERMKER_UNAME=Linux 6.1.0-18-amd64\r\n\
             NAME=\"Debian GNU/Linux\"\r\nVERSION_ID=\"12\"\r\nID=debian\r\n",
        )
        .unwrap();
        assert_eq!(debian.title(), "Debian 12");

        let arch =
            parse("TERMKER_UNAME=Linux 6.11.5-arch1-1\nNAME='Arch Linux'\nID=arch\n").unwrap();
        assert_eq!(arch.title(), "Arch Linux");
        assert_eq!(arch.family_text(), "Linux 6.11");
    }

    #[test]
    fn macos_comes_from_sw_vers() {
        let os = parse(
            "TERMKER_UNAME=Darwin 24.1.0\n\
             ProductName:\t\tmacOS\nProductVersion:\t\t15.1\nBuildVersion:\t\t24B83\n",
        )
        .unwrap();
        assert_eq!(os.family, OsFamily::MacOs);
        assert_eq!(os.id, "macos");
        assert_eq!(os.title(), "macOS 15.1");
        assert_eq!(os.family_text(), "Mac");
    }

    #[test]
    fn bsd_and_other_unix_fall_back_to_uname() {
        let openbsd = parse("TERMKER_UNAME=OpenBSD 7.6\n").unwrap();
        assert_eq!(openbsd.family, OsFamily::Unix);
        assert_eq!(openbsd.id, "openbsd");
        assert_eq!(openbsd.title(), "OpenBSD 7.6");
        assert_eq!(openbsd.family_text(), "Unix");

        let freebsd = parse(
            "TERMKER_UNAME=FreeBSD 14.1-RELEASE\nNAME=FreeBSD\nVERSION_ID=\"14.1\"\nID=freebsd\n",
        )
        .unwrap();
        assert_eq!(freebsd.title(), "FreeBSD 14.1");
        assert_eq!(freebsd.family_text(), "Unix");

        let plain = parse("TERMKER_UNAME=Linux 5.10.0\n").unwrap();
        assert_eq!(plain.id, "linux");
        assert_eq!(plain.title(), "Linux 5.10.0");
        assert_eq!(plain.family_text(), "Linux");
    }

    #[test]
    fn answers_that_are_not_unix_give_nothing() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("TERMKER_UNAME=\n"), None);
        assert_eq!(
            parse("'sh' is not recognized as an internal or external command"),
            None
        );
        // A banner that happens to contain `=` is not an operating system.
        assert_eq!(parse("status=ok\nNAME=router\n"), None);
    }

    #[test]
    fn hostile_text_is_cut_short_and_made_printable() {
        let long = "x".repeat(500);
        let os = parse(&format!(
            "TERMKER_UNAME=Linux 6.8.0\nNAME=\"Evil\u{1b}[31m{long}\"\nID=evil\n"
        ))
        .unwrap();
        assert!(!os.name.contains('\u{1b}'));
        assert_eq!(os.name.chars().count(), MAX_FIELD_CHARS);
    }
}
