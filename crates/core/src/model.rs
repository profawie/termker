//! Persisted data model: hosts, forwards, keychain entries and settings.

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
    /// Another host to go through first to reach this one (ProxyJump).
    #[serde(default)]
    pub jump_host: Option<Uuid>,
    #[serde(default)]
    pub auth: AuthMethod,
    /// A keychain identity to sign in with. When set, it replaces `username`
    /// and `auth`.
    #[serde(default)]
    pub identity: Option<Uuid>,
}

/// A named group of hosts on the Hosts page. Hosts refer to it by name
/// (`Host::group`); a group can exist without any hosts in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// Whether its hosts are hidden on the Hosts page.
    #[serde(default)]
    pub collapsed: bool,
}

/// How the user authenticates to a host. The secret itself is not stored
/// here: passwords are asked for when connecting, and keys stay in their file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthMethod {
    #[default]
    Password,
    /// A private key file somewhere on this computer.
    KeyFile { path: String },
    /// A key kept in the keychain.
    Key { key: Uuid },
}

/// A key pair in the keychain. Only public material is kept here; the
/// private key lives in its own file in the keys folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyEntry {
    pub id: Uuid,
    pub name: String,
    /// Display name of the algorithm, e.g. `ED25519` or `RSA 4096`.
    pub algorithm: String,
    /// Public key as an `authorized_keys` line.
    pub public_key: String,
    /// SHA-256 fingerprint of the public key.
    pub fingerprint: String,
    /// Whether the private key file is protected by a passphrase.
    #[serde(default)]
    pub encrypted: bool,
    /// OpenSSH user certificate for this key, as a `*-cert.pub` line.
    #[serde(default)]
    pub certificate: Option<String>,
}

/// A reusable sign-in: a username with a key or a password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub id: Uuid,
    pub name: String,
    pub username: String,
    /// Keychain key to sign in with; `None` means password.
    #[serde(default)]
    pub key: Option<Uuid>,
}

impl Default for Identity {
    fn default() -> Self {
        Self::new()
    }
}

impl Identity {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            username: String::new(),
            key: None,
        }
    }
}

fn default_port() -> u16 {
    DEFAULT_SSH_PORT
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
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
            auth: AuthMethod::Password,
            identity: None,
        }
    }

    /// Display name: the label if set, otherwise the address.
    pub fn display_name(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.address
        } else {
            &self.label
        }
    }

    /// `user@address:port`, leaving out the parts that are not needed.
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

/// Light or dark look for the app's windows, lists and dialogs. The terminal
/// has its own color theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    /// Follow the operating system's light or dark setting.
    System,
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub appearance: Appearance,
    /// Name of the terminal color theme. Saved as `theme` before the app
    /// had a separate light and dark mode.
    #[serde(default = "default_theme", alias = "theme")]
    pub terminal_theme: String,
    #[serde(default = "default_font_size")]
    pub terminal_font_size: f32,
}

fn default_theme() -> String {
    crate::DEFAULT_THEME.to_string()
}

fn default_font_size() -> f32 {
    14.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            terminal_theme: default_theme(),
            terminal_font_size: default_font_size(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardKind {
    /// Listen on this computer, connect out from the SSH host (`ssh -L`).
    Local,
    /// Listen on the SSH host, connect back to this computer (`ssh -R`).
    Remote,
    /// SOCKS5 proxy on this computer (`ssh -D`).
    Dynamic,
}

impl ForwardKind {
    pub const ALL: [ForwardKind; 3] = [Self::Local, Self::Remote, Self::Dynamic];

    pub fn title(self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::Remote => "Remote",
            Self::Dynamic => "Dynamic (SOCKS5)",
        }
    }
}

pub const LOOPBACK: &str = "127.0.0.1";

/// A saved port forwarding rule through one of the hosts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Forward {
    pub id: Uuid,
    #[serde(default)]
    pub label: String,
    pub host_id: Uuid,
    pub kind: ForwardKind,
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
    pub bind_port: u16,
    /// Not used for dynamic forwards.
    #[serde(default)]
    pub dest_host: String,
    #[serde(default)]
    pub dest_port: u16,
}

fn default_bind_address() -> String {
    LOOPBACK.to_string()
}

impl Forward {
    pub fn new(host_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            label: String::new(),
            host_id,
            kind: ForwardKind::Local,
            bind_address: default_bind_address(),
            bind_port: 0,
            dest_host: String::new(),
            dest_port: 0,
        }
    }

    /// Whether the listening side accepts connections only from the machine
    /// it listens on.
    pub fn binds_loopback(&self) -> bool {
        matches!(
            self.bind_address.as_str(),
            "127.0.0.1" | "localhost" | "::1"
        )
    }

    /// One sentence saying what the rule does, with `host` as the SSH host's
    /// name.
    pub fn describe(&self, host: &str) -> String {
        let bind = format!("{}:{}", self.bind_address, self.bind_port);
        let dest = format!("{}:{}", self.dest_host, self.dest_port);
        match self.kind {
            ForwardKind::Local => {
                format!("Connections to {bind} on this computer go through {host} to {dest}.")
            }
            ForwardKind::Remote => {
                format!("Connections to {bind} on {host} come back to {dest} from this computer.")
            }
            ForwardKind::Dynamic => {
                format!("SOCKS5 proxy on {bind}. Apps that use it reach the network from {host}.")
            }
        }
    }

    /// Short form, e.g. `127.0.0.1:8080 -> db:5432`.
    pub fn summary(&self) -> String {
        let bind = format!("{}:{}", self.bind_address, self.bind_port);
        match self.kind {
            ForwardKind::Local => format!("{bind} -> {}:{}", self.dest_host, self.dest_port),
            ForwardKind::Remote => {
                format!("{bind} (remote) -> {}:{}", self.dest_host, self.dest_port)
            }
            ForwardKind::Dynamic => format!("{bind} (SOCKS5)"),
        }
    }

    pub fn display_name(&self) -> String {
        if self.label.is_empty() {
            self.summary()
        } else {
            self.label.clone()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Data {
    #[serde(default)]
    pub hosts: Vec<Host>,
    /// Host groups, kept sorted by name.
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub forwards: Vec<Forward>,
    #[serde(default)]
    pub keys: Vec<KeyEntry>,
    #[serde(default)]
    pub identities: Vec<Identity>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, PartialEq)]
pub enum HostError {
    MissingAddress,
    MissingKeyPath,
    InvalidPort,
    JumpLoop,
    MissingHost,
    MissingBindAddress,
    MissingDestination,
    MissingKey,
    MissingName,
    MissingUsername,
    GroupExists,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MissingAddress => "Address is required.",
            Self::GroupExists => "A group with this name already exists.",
            Self::MissingKeyPath => "Choose the private key file to use.",
            Self::InvalidPort => "Port must be a number from 1 to 65535.",
            Self::JumpLoop => "This jump host would create a loop.",
            Self::MissingHost => "Choose the host to forward through.",
            Self::MissingBindAddress => "Listen address is required.",
            Self::MissingDestination => "Destination host and port are required.",
            Self::MissingKey => "Choose a key from the keychain.",
            Self::MissingName => "Name is required.",
            Self::MissingUsername => "Username is required.",
        })
    }
}

impl Data {
    pub fn host(&self, id: Uuid) -> Option<&Host> {
        self.hosts.iter().find(|host| host.id == id)
    }

    pub fn key(&self, id: Uuid) -> Option<&KeyEntry> {
        self.keys.iter().find(|key| key.id == id)
    }

    pub fn identity(&self, id: Uuid) -> Option<&Identity> {
        self.identities.iter().find(|identity| identity.id == id)
    }

    /// Username and sign-in method for `host`, taken from its identity when
    /// it has one. An identity that no longer exists is ignored.
    pub fn credentials(&self, host: &Host) -> (String, AuthMethod) {
        match host.identity.and_then(|id| self.identity(id)) {
            Some(identity) => (
                identity.username.clone(),
                match identity.key {
                    Some(key) => AuthMethod::Key { key },
                    None => AuthMethod::Password,
                },
            ),
            None => (host.username.clone(), host.auth.clone()),
        }
    }

    /// `user@address:port` for `host`, with the username it really signs in
    /// with.
    pub fn target(&self, host: &Host) -> String {
        let (username, _) = self.credentials(host);
        Host {
            username,
            ..host.clone()
        }
        .target()
    }

    /// Number of hosts and identities that sign in with this key.
    pub fn key_usage(&self, id: Uuid) -> usize {
        let hosts = self
            .hosts
            .iter()
            .filter(|host| host.auth == AuthMethod::Key { key: id })
            .count();
        let identities = self
            .identities
            .iter()
            .filter(|identity| identity.key == Some(id))
            .count();
        hosts + identities
    }

    /// Number of hosts that sign in with this identity.
    pub fn identity_usage(&self, id: Uuid) -> usize {
        self.hosts
            .iter()
            .filter(|host| host.identity == Some(id))
            .count()
    }

    /// Insert a new identity or update an existing one, after validation.
    pub fn upsert_identity(&mut self, mut identity: Identity) -> Result<(), HostError> {
        identity.name = identity.name.trim().to_string();
        identity.username = identity.username.trim().to_string();
        if identity.name.is_empty() {
            return Err(HostError::MissingName);
        }
        if identity.username.is_empty() {
            return Err(HostError::MissingUsername);
        }
        if identity.key.is_some_and(|key| self.key(key).is_none()) {
            return Err(HostError::MissingKey);
        }
        match self
            .identities
            .iter_mut()
            .find(|existing| existing.id == identity.id)
        {
            Some(existing) => *existing = identity,
            None => self.identities.push(identity),
        }
        Ok(())
    }

    /// Delete an identity. Hosts that used it go back to their own username
    /// and sign-in method.
    pub fn remove_identity(&mut self, id: Uuid) {
        self.identities.retain(|identity| identity.id != id);
        for host in &mut self.hosts {
            if host.identity == Some(id) {
                host.identity = None;
            }
        }
    }

    /// Delete a key entry. Hosts and identities that used it fall back to
    /// password sign-in.
    pub fn remove_key(&mut self, id: Uuid) {
        self.keys.retain(|key| key.id != id);
        for host in &mut self.hosts {
            if host.auth == (AuthMethod::Key { key: id }) {
                host.auth = AuthMethod::Password;
            }
        }
        for identity in &mut self.identities {
            if identity.key == Some(id) {
                identity.key = None;
            }
        }
    }

    /// Connection path to `host`: outermost jump host first, `host` last.
    ///
    /// `host` does not have to be saved yet, so the form can check a draft
    /// before saving it.
    pub fn connection_path<'a>(&'a self, host: &'a Host) -> Result<Vec<&'a Host>, HostError> {
        let mut path = vec![host];
        let mut next = host.jump_host;
        while let Some(id) = next {
            if path.iter().any(|seen| seen.id == id) {
                return Err(HostError::JumpLoop);
            }
            // A jump host that no longer exists is ignored.
            let Some(jump) = self.host(id) else { break };
            path.push(jump);
            next = jump.jump_host;
        }
        path.reverse();
        Ok(path)
    }

    /// Insert a new host or update an existing one, after validation.
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
        if let AuthMethod::KeyFile { path } = &mut host.auth {
            *path = path.trim().to_string();
            if path.is_empty() {
                return Err(HostError::MissingKeyPath);
            }
        }
        if let AuthMethod::Key { key } = &host.auth
            && self.key(*key).is_none()
        {
            return Err(HostError::MissingKey);
        }
        if host.identity.is_some_and(|id| self.identity(id).is_none()) {
            host.identity = None;
        }
        self.connection_path(&host)?;
        // A group typed in a different case is the existing group.
        if let Some(group) = self.group(&host.group) {
            host.group = group.name.clone();
        }
        match self
            .hosts
            .iter_mut()
            .find(|existing| existing.id == host.id)
        {
            Some(existing) => *existing = host,
            None => self.hosts.push(host),
        }
        self.sync_groups();
        Ok(())
    }

    /// The group called `name`, compared without regard to case.
    pub fn group(&self, name: &str) -> Option<&Group> {
        let name = name.trim().to_lowercase();
        self.groups
            .iter()
            .find(|group| group.name.to_lowercase() == name)
    }

    /// Hosts in the group `name`; an empty name means hosts without a group.
    pub fn hosts_in_group(&self, name: &str) -> usize {
        self.hosts.iter().filter(|host| host.group == name).count()
    }

    /// Make sure every group a host refers to exists, and keep the groups
    /// sorted. Data saved before groups were stored on their own only has
    /// the name on each host.
    pub fn sync_groups(&mut self) {
        for index in 0..self.hosts.len() {
            let name = self.hosts[index].group.trim().to_string();
            match self.group(&name).map(|group| group.name.clone()) {
                Some(existing) => self.hosts[index].group = existing,
                None if name.is_empty() => self.hosts[index].group = name,
                None => {
                    self.groups.push(Group {
                        name: name.clone(),
                        collapsed: false,
                    });
                    self.hosts[index].group = name;
                }
            }
        }
        self.groups.retain(|group| !group.name.trim().is_empty());
        self.groups
            .sort_by_key(|group| (group.name.to_lowercase(), group.name.clone()));
        self.groups.dedup_by_key(|group| group.name.to_lowercase());
    }

    /// Create an empty group.
    pub fn add_group(&mut self, name: &str) -> Result<(), HostError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(HostError::MissingName);
        }
        if self.group(name).is_some() {
            return Err(HostError::GroupExists);
        }
        self.groups.push(Group {
            name: name.to_string(),
            collapsed: false,
        });
        self.sync_groups();
        Ok(())
    }

    /// Rename a group; its hosts stay in it.
    pub fn rename_group(&mut self, from: &str, to: &str) -> Result<(), HostError> {
        let to = to.trim();
        if to.is_empty() {
            return Err(HostError::MissingName);
        }
        // Changing only the letter case of the same group is allowed.
        if to.to_lowercase() != from.to_lowercase() && self.group(to).is_some() {
            return Err(HostError::GroupExists);
        }
        for group in &mut self.groups {
            if group.name == from {
                group.name = to.to_string();
            }
        }
        for host in &mut self.hosts {
            if host.group == from {
                host.group = to.to_string();
            }
        }
        self.sync_groups();
        Ok(())
    }

    /// Delete a group. Its hosts are kept and become ungrouped.
    pub fn remove_group(&mut self, name: &str) {
        self.groups.retain(|group| group.name != name);
        for host in &mut self.hosts {
            if host.group == name {
                host.group.clear();
            }
        }
    }

    /// Show or hide a group's hosts on the Hosts page.
    pub fn set_group_collapsed(&mut self, name: &str, collapsed: bool) {
        for group in &mut self.groups {
            if group.name == name {
                group.collapsed = collapsed;
            }
        }
    }

    /// Move a host to another group; an empty name removes it from its group.
    pub fn move_host(&mut self, id: Uuid, group: &str) {
        let group = self
            .group(group)
            .map(|group| group.name.clone())
            .unwrap_or_default();
        if let Some(host) = self.hosts.iter_mut().find(|host| host.id == id) {
            host.group = group;
        }
    }

    /// Insert a new forward or update an existing one, after validation.
    pub fn upsert_forward(&mut self, mut forward: Forward) -> Result<(), HostError> {
        forward.label = forward.label.trim().to_string();
        forward.bind_address = forward.bind_address.trim().to_string();
        forward.dest_host = forward.dest_host.trim().to_string();
        if self.host(forward.host_id).is_none() {
            return Err(HostError::MissingHost);
        }
        if forward.bind_address.is_empty() {
            return Err(HostError::MissingBindAddress);
        }
        if forward.bind_port == 0 {
            return Err(HostError::InvalidPort);
        }
        if forward.kind == ForwardKind::Dynamic {
            forward.dest_host.clear();
            forward.dest_port = 0;
        } else if forward.dest_host.is_empty() || forward.dest_port == 0 {
            return Err(HostError::MissingDestination);
        }
        match self
            .forwards
            .iter_mut()
            .find(|existing| existing.id == forward.id)
        {
            Some(existing) => *existing = forward,
            None => self.forwards.push(forward),
        }
        Ok(())
    }

    pub fn remove_forward(&mut self, id: Uuid) {
        self.forwards.retain(|forward| forward.id != id);
    }

    /// Delete a host, its forwards, and jump host references to it.
    pub fn remove_host(&mut self, id: Uuid) {
        self.hosts.retain(|host| host.id != id);
        self.forwards.retain(|forward| forward.host_id != id);
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
    fn settings_from_before_light_and_dark_mode_keep_the_terminal_theme() {
        let old: Settings = serde_json::from_str(r#"{"theme":"Amber"}"#).unwrap();
        assert_eq!(old.terminal_theme, "Amber");
        assert_eq!(old.appearance, Appearance::Dark);
        let text = serde_json::to_string(&old).unwrap();
        assert!(text.contains(r#""terminal_theme":"Amber""#));
        assert!(text.contains(r#""appearance":"dark""#));
    }

    fn grouped(label: &str, group: &str) -> Host {
        Host {
            group: group.into(),
            ..host(label)
        }
    }

    fn group_names(data: &Data) -> Vec<&str> {
        data.groups
            .iter()
            .map(|group| group.name.as_str())
            .collect()
    }

    #[test]
    fn groups_from_before_they_were_stored_are_picked_up_from_hosts() {
        let old = serde_json::json!({
            "hosts": [
                {"id": Uuid::new_v4(), "label": "web", "address": "w", "group": "Prod"},
                {"id": Uuid::new_v4(), "label": "db", "address": "d", "group": " prod "},
                {"id": Uuid::new_v4(), "label": "lab", "address": "l"}
            ]
        });
        let mut data: Data = serde_json::from_value(old).unwrap();
        assert!(data.groups.is_empty());
        data.sync_groups();
        assert_eq!(group_names(&data), ["Prod"]);
        assert_eq!(data.hosts_in_group("Prod"), 2);
        assert_eq!(data.hosts_in_group(""), 1);
    }

    #[test]
    fn saving_a_host_creates_its_group_and_reuses_one_in_another_case() {
        let mut data = Data::default();
        data.upsert_host(grouped("web", "Prod")).unwrap();
        data.upsert_host(grouped("db", "PROD")).unwrap();
        data.upsert_host(grouped("lab", "alpha")).unwrap();
        assert_eq!(group_names(&data), ["alpha", "Prod"]);
        assert_eq!(data.hosts_in_group("Prod"), 2);
    }

    #[test]
    fn groups_can_be_added_renamed_and_deleted() {
        let mut data = Data::default();
        let web = grouped("web", "Prod");
        data.upsert_host(web.clone()).unwrap();

        data.add_group(" Staging ").unwrap();
        assert_eq!(group_names(&data), ["Prod", "Staging"]);
        assert_eq!(data.add_group("staging"), Err(HostError::GroupExists));
        assert_eq!(data.add_group("  "), Err(HostError::MissingName));

        assert_eq!(
            data.rename_group("Prod", "staging"),
            Err(HostError::GroupExists)
        );
        data.rename_group("Prod", "Production").unwrap();
        data.rename_group("Production", "production").unwrap();
        assert_eq!(group_names(&data), ["production", "Staging"]);
        assert_eq!(data.host(web.id).unwrap().group, "production");

        data.set_group_collapsed("Staging", true);
        assert!(data.group("staging").unwrap().collapsed);

        data.move_host(web.id, "STAGING");
        assert_eq!(data.host(web.id).unwrap().group, "Staging");

        data.remove_group("Staging");
        assert_eq!(group_names(&data), ["production"]);
        // The host is kept, without a group.
        assert_eq!(data.host(web.id).unwrap().group, "");
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
    fn key_file_auth_needs_a_path_and_round_trips() {
        let mut data = Data::default();
        let mut h = host("k");
        h.auth = AuthMethod::KeyFile { path: "  ".into() };
        assert_eq!(data.upsert_host(h.clone()), Err(HostError::MissingKeyPath));
        h.auth = AuthMethod::KeyFile {
            path: " ~/.ssh/id_ed25519 ".into(),
        };
        data.upsert_host(h.clone()).unwrap();
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains(r#""auth":{"type":"key_file","path":"~/.ssh/id_ed25519"}"#));
        assert_eq!(serde_json::from_str::<Data>(&json).unwrap(), data);
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

    fn forward(data: &mut Data) -> Forward {
        let h = host("fw");
        data.upsert_host(h.clone()).unwrap();
        Forward {
            bind_port: 8080,
            dest_host: "db.internal".into(),
            dest_port: 5432,
            ..Forward::new(h.id)
        }
    }

    #[test]
    fn forward_validation() {
        let mut data = Data::default();
        let good = forward(&mut data);
        data.upsert_forward(good.clone()).unwrap();

        let unknown_host = Forward {
            host_id: Uuid::new_v4(),
            ..good.clone()
        };
        assert_eq!(
            data.upsert_forward(unknown_host),
            Err(HostError::MissingHost)
        );
        let no_port = Forward {
            bind_port: 0,
            ..good.clone()
        };
        assert_eq!(data.upsert_forward(no_port), Err(HostError::InvalidPort));
        let no_bind = Forward {
            bind_address: " ".into(),
            ..good.clone()
        };
        assert_eq!(
            data.upsert_forward(no_bind),
            Err(HostError::MissingBindAddress)
        );
        let no_dest = Forward {
            dest_host: String::new(),
            ..good.clone()
        };
        assert_eq!(
            data.upsert_forward(no_dest),
            Err(HostError::MissingDestination)
        );

        // A dynamic forward needs no destination, and drops a stale one.
        let dynamic = Forward {
            kind: ForwardKind::Dynamic,
            ..good.clone()
        };
        data.upsert_forward(dynamic).unwrap();
        assert_eq!(data.forwards.len(), 1);
        assert_eq!(data.forwards[0].dest_host, "");
        assert_eq!(data.forwards[0].summary(), "127.0.0.1:8080 (SOCKS5)");
    }

    #[test]
    fn forward_text_and_loopback_check() {
        let mut data = Data::default();
        let mut f = forward(&mut data);
        assert_eq!(f.summary(), "127.0.0.1:8080 -> db.internal:5432");
        assert_eq!(
            f.describe("fw"),
            "Connections to 127.0.0.1:8080 on this computer go through fw to db.internal:5432."
        );
        assert!(f.binds_loopback());
        f.bind_address = "0.0.0.0".into();
        assert!(!f.binds_loopback());
        f.kind = ForwardKind::Remote;
        assert!(
            f.describe("fw")
                .starts_with("Connections to 0.0.0.0:8080 on fw come back")
        );
    }

    #[test]
    fn removing_a_host_removes_its_forwards() {
        let mut data = Data::default();
        let f = forward(&mut data);
        data.upsert_forward(f.clone()).unwrap();
        data.remove_host(f.host_id);
        assert!(data.forwards.is_empty());
    }

    fn key_entry(name: &str) -> KeyEntry {
        KeyEntry {
            id: Uuid::new_v4(),
            name: name.into(),
            algorithm: "ED25519".into(),
            public_key: "ssh-ed25519 AAAA".into(),
            fingerprint: "SHA256:x".into(),
            encrypted: false,
            certificate: None,
        }
    }

    #[test]
    fn identity_replaces_host_username_and_auth() {
        let mut data = Data::default();
        let key = key_entry("work");
        data.keys.push(key.clone());
        let identity = Identity {
            name: " Deploy ".into(),
            username: " deploy ".into(),
            key: Some(key.id),
            ..Identity::new()
        };
        data.upsert_identity(identity.clone()).unwrap();
        assert_eq!(data.identities[0].name, "Deploy");

        let mut h = host("a");
        h.username = "root".into();
        assert_eq!(
            data.credentials(&h),
            ("root".to_string(), AuthMethod::Password)
        );
        h.identity = Some(identity.id);
        assert_eq!(
            data.credentials(&h),
            ("deploy".to_string(), AuthMethod::Key { key: key.id })
        );
        // An identity that was deleted is ignored.
        h.identity = Some(Uuid::new_v4());
        assert_eq!(data.credentials(&h).0, "root");
    }

    #[test]
    fn identity_validation() {
        let mut data = Data::default();
        let mut identity = Identity::new();
        assert_eq!(
            data.upsert_identity(identity.clone()),
            Err(HostError::MissingName)
        );
        identity.name = "x".into();
        assert_eq!(
            data.upsert_identity(identity.clone()),
            Err(HostError::MissingUsername)
        );
        identity.username = "u".into();
        identity.key = Some(Uuid::new_v4());
        assert_eq!(
            data.upsert_identity(identity.clone()),
            Err(HostError::MissingKey)
        );
        identity.key = None;
        data.upsert_identity(identity).unwrap();
    }

    #[test]
    fn removing_a_key_or_identity_clears_references() {
        let mut data = Data::default();
        let key = key_entry("k");
        data.keys.push(key.clone());
        let identity = Identity {
            name: "i".into(),
            username: "u".into(),
            key: Some(key.id),
            ..Identity::new()
        };
        data.upsert_identity(identity.clone()).unwrap();
        let mut with_key = host("a");
        with_key.auth = AuthMethod::Key { key: key.id };
        data.upsert_host(with_key).unwrap();
        let mut with_identity = host("b");
        with_identity.identity = Some(identity.id);
        data.upsert_host(with_identity).unwrap();
        assert_eq!(data.key_usage(key.id), 2);
        assert_eq!(data.identity_usage(identity.id), 1);

        data.remove_key(key.id);
        assert_eq!(data.hosts[0].auth, AuthMethod::Password);
        assert_eq!(data.identities[0].key, None);

        data.remove_identity(identity.id);
        assert_eq!(data.hosts[1].identity, None);
    }

    #[test]
    fn host_cannot_use_a_key_that_does_not_exist() {
        let mut data = Data::default();
        let mut h = host("a");
        h.auth = AuthMethod::Key {
            key: Uuid::new_v4(),
        };
        assert_eq!(data.upsert_host(h), Err(HostError::MissingKey));
    }

    #[test]
    fn old_files_without_new_fields_still_load() {
        let json = r#"{"hosts":[{"id":"6f0c3b1e-58c4-4a57-9d0b-1f4f3f0f6b11","label":"a","address":"a"}]}"#;
        let data: Data = serde_json::from_str(json).unwrap();
        assert_eq!(data.hosts[0].port, 22);
        assert_eq!(data.hosts[0].auth, AuthMethod::Password);
        assert!(data.forwards.is_empty());
        assert_eq!(data.settings, Settings::default());
    }
}
