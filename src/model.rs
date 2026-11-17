//! Model data yang disimpan: host dan tetapan.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_SSH_PORT: u16 = 22;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Host {
    pub id: Uuid,
    pub label: String,
    pub address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub group: String,
    /// Host lain yang dilalui dahulu untuk sampai ke host ini (ProxyJump).
    #[serde(default)]
    pub jump_host: Option<Uuid>,
}

fn default_port() -> u16 {
    DEFAULT_SSH_PORT
}

impl Host {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            label: String::new(),
            address: String::new(),
            port: DEFAULT_SSH_PORT,
            username: String::new(),
            group: String::new(),
            jump_host: None,
        }
    }

    /// Nama paparan: label jika ada, jika tidak alamat.
    pub fn display_name(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.address
        } else {
            &self.label
        }
    }

    /// `user@alamat:port`, dengan bahagian yang tidak perlu ditinggalkan.
    pub fn target(&self) -> String {
        let mut out = String::new();
        if !self.username.is_empty() {
            out.push_str(&self.username);
            out.push('@');
        }
        out.push_str(&self.address);
        if self.port != DEFAULT_SSH_PORT {
            out.push_str(&format!(":{}", self.port));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_font_size")]
    pub terminal_font_size: f32,
}

fn default_theme() -> String {
    crate::theme::DEFAULT_THEME.to_string()
}

fn default_font_size() -> f32 {
    14.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            terminal_font_size: default_font_size(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Data {
    #[serde(default)]
    pub hosts: Vec<Host>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, PartialEq)]
pub enum HostError {
    MissingAddress,
    InvalidPort,
    JumpLoop,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MissingAddress => "Address is required.",
            Self::InvalidPort => "Port must be a number from 1 to 65535.",
            Self::JumpLoop => "This jump host would create a loop.",
        })
    }
}

impl Data {
    pub fn host(&self, id: Uuid) -> Option<&Host> {
        self.hosts.iter().find(|host| host.id == id)
    }

    /// Laluan sambungan ke `host`: jump host paling luar dahulu, `host` terakhir.
    ///
    /// `host` tidak semestinya sudah disimpan, supaya borang boleh menyemak
    /// draf sebelum menyimpannya.
    pub fn connection_path<'a>(&'a self, host: &'a Host) -> Result<Vec<&'a Host>, HostError> {
        let mut path = vec![host];
        let mut next = host.jump_host;
        while let Some(id) = next {
            if path.iter().any(|seen| seen.id == id) {
                return Err(HostError::JumpLoop);
            }
            // Jump host yang sudah dipadam diabaikan.
            let Some(jump) = self.host(id) else { break };
            path.push(jump);
            next = jump.jump_host;
        }
        path.reverse();
        Ok(path)
    }

    /// Simpan host baharu atau kemas kini yang sedia ada selepas disahkan.
    pub fn upsert_host(&mut self, mut host: Host) -> Result<(), HostError> {
        host.label = host.label.trim().to_string();
        host.address = host.address.trim().to_string();
        host.username = host.username.trim().to_string();
        host.group = host.group.trim().to_string();
        if host.address.is_empty() {
            return Err(HostError::MissingAddress);
        }
        if host.port == 0 {
            return Err(HostError::InvalidPort);
        }
        self.connection_path(&host)?;
        match self
            .hosts
            .iter_mut()
            .find(|existing| existing.id == host.id)
        {
            Some(existing) => *existing = host,
            None => self.hosts.push(host),
        }
        Ok(())
    }

    /// Padam host dan buang rujukan jump host kepadanya.
    pub fn remove_host(&mut self, id: Uuid) {
        self.hosts.retain(|host| host.id != id);
        for host in &mut self.hosts {
            if host.jump_host == Some(id) {
                host.jump_host = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(label: &str) -> Host {
        Host {
            label: label.into(),
            address: format!("{label}.example.com"),
            ..Host::new()
        }
    }

    #[test]
    fn target_omits_default_port_and_empty_user() {
        let mut h = host("a");
        assert_eq!(h.target(), "a.example.com");
        h.username = "root".into();
        h.port = 2222;
        assert_eq!(h.target(), "root@a.example.com:2222");
    }

    #[test]
    fn connection_path_orders_outermost_jump_first() {
        let bastion = host("bastion");
        let mut middle = host("middle");
        middle.jump_host = Some(bastion.id);
        let mut target = host("target");
        target.jump_host = Some(middle.id);
        let data = Data {
            hosts: vec![bastion, middle, target.clone()],
            ..Default::default()
        };
        let labels: Vec<_> = data
            .connection_path(&target)
            .unwrap()
            .iter()
            .map(|h| h.label.as_str())
            .collect();
        assert_eq!(labels, ["bastion", "middle", "target"]);
    }

    #[test]
    fn upsert_rejects_jump_loops() {
        let mut data = Data::default();
        let a = host("a");
        let mut b = host("b");
        b.jump_host = Some(a.id);
        data.upsert_host(a.clone()).unwrap();
        data.upsert_host(b.clone()).unwrap();

        let mut a_via_b = a.clone();
        a_via_b.jump_host = Some(b.id);
        assert_eq!(data.upsert_host(a_via_b), Err(HostError::JumpLoop));

        let mut a_via_self = a.clone();
        a_via_self.jump_host = Some(a.id);
        assert_eq!(data.upsert_host(a_via_self), Err(HostError::JumpLoop));
        assert_eq!(data.host(a.id).unwrap().jump_host, None);
    }

    #[test]
    fn upsert_validates_and_trims() {
        let mut data = Data::default();
        let mut h = host("x");
        h.address = "   ".into();
        assert_eq!(data.upsert_host(h.clone()), Err(HostError::MissingAddress));
        h.address = " 10.0.0.1 ".into();
        h.port = 0;
        assert_eq!(data.upsert_host(h.clone()), Err(HostError::InvalidPort));
        h.port = 22;
        data.upsert_host(h.clone()).unwrap();
        assert_eq!(data.host(h.id).unwrap().address, "10.0.0.1");
    }

    #[test]
    fn remove_clears_jump_references() {
        let mut data = Data::default();
        let a = host("a");
        let mut b = host("b");
        b.jump_host = Some(a.id);
        data.upsert_host(a.clone()).unwrap();
        data.upsert_host(b.clone()).unwrap();
        data.remove_host(a.id);
        assert_eq!(data.hosts.len(), 1);
        assert_eq!(data.hosts[0].jump_host, None);
    }

    #[test]
    fn old_files_without_new_fields_still_load() {
        let json = r#"{"hosts":[{"id":"6f0c3b1e-58c4-4a57-9d0b-1f4f3f0f6b11","label":"a","address":"a"}]}"#;
        let data: Data = serde_json::from_str(json).unwrap();
        assert_eq!(data.hosts[0].port, 22);
        assert_eq!(data.settings, Settings::default());
    }
}
