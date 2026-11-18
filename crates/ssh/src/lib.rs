//! SSH connections: host key verification, user authentication, jump hosts
//! (ProxyJump) and interactive shell sessions. Built on `russh`.

pub mod forward;
pub mod known_hosts;
pub mod sftp;
pub mod socks;

use std::{fmt, io, path::PathBuf, sync::Arc, time::Duration};

use russh::{
    ChannelMsg, Disconnect,
    client::{self, Handle},
    keys::{Certificate, PrivateKeyWithHashAlg, PublicKey, load_secret_key},
};
use tokio::sync::{mpsc, oneshot};

pub use known_hosts::{HostKeyStatus, KnownHosts};
pub use termker_core::secrets::SecretStore;
use termker_core::{
    keychain::KeyStore,
    model::{AuthMethod, Data, Host},
    secrets,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_SECRET_ATTEMPTS: usize = 3;

/// One hop in a connection path.
#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth: HopAuth,
}

/// How to sign in at one hop, with everything already looked up.
#[derive(Debug, Clone, PartialEq)]
pub enum HopAuth {
    Password,
    Key {
        /// Private key file.
        path: String,
        /// What to call the key in prompts and errors.
        name: String,
        /// OpenSSH user certificate to present with the key.
        certificate: Option<String>,
    },
}

/// Hops for the connection path to `host`, outermost jump host first, with
/// identities and keychain keys resolved.
pub fn hops(data: &Data, host: &Host, keys: &KeyStore) -> Result<Vec<Hop>, String> {
    let path = data.connection_path(host).map_err(|err| err.to_string())?;
    path.into_iter()
        .map(|host| {
            let (username, auth) = data.credentials(host);
            let auth = match auth {
                AuthMethod::Password => HopAuth::Password,
                AuthMethod::KeyFile { path } => HopAuth::Key {
                    name: path.clone(),
                    path,
                    certificate: None,
                },
                AuthMethod::Key { key } => {
                    let entry = data.key(key).ok_or_else(|| {
                        format!(
                            "The key for {} is no longer in the keychain.",
                            host.display_name()
                        )
                    })?;
                    HopAuth::Key {
                        path: keys.path(entry.id).display().to_string(),
                        name: entry.name.clone(),
                        certificate: entry.certificate.clone(),
                    }
                }
            };
            Ok(Hop {
                name: host.display_name().to_string(),
                address: host.address.clone(),
                port: host.port,
                username,
                auth,
            })
        })
        .collect()
}

#[derive(Debug)]
pub enum SshError {
    Protocol(russh::Error),
    Io(io::Error),
    Timeout {
        hop: String,
    },
    MissingUsername {
        hop: String,
    },
    HostKeyRejected {
        hop: String,
    },
    Key {
        path: String,
        reason: String,
    },
    AuthFailed {
        hop: String,
    },
    /// The jump host `via` would not open the tunnel to the next hop.
    JumpRefused {
        via: String,
        hop: String,
        address: String,
        port: u16,
        reason: russh::ChannelOpenFailure,
    },
    /// A port forward could not be set up; the text says why.
    Forward(String),
    /// An SFTP session could not be set up or was lost; the text says why.
    Sftp(String),
    Cancelled,
}

impl fmt::Display for SshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(err) => write!(f, "SSH error: {err}"),
            Self::Io(err) => write!(f, "Network error: {err}"),
            Self::Timeout { hop } => write!(f, "Timed out connecting to {hop}."),
            Self::MissingUsername { hop } => {
                write!(f, "{hop} has no username. Edit the host and add one.")
            }
            Self::HostKeyRejected { hop } => {
                write!(f, "The host key for {hop} was not trusted.")
            }
            Self::Key { path, reason } => write!(f, "Could not use key {path}: {reason}"),
            Self::AuthFailed { hop } => write!(f, "{hop} rejected the login."),
            Self::JumpRefused {
                via,
                hop,
                address,
                port,
                reason,
            } => {
                use russh::ChannelOpenFailure as Reason;
                match reason {
                    Reason::ConnectFailed => write!(
                        f,
                        "{via} could not reach {hop} at {address}:{port}. The jump host \
                         connects to this address itself, so it must be an address and port \
                         that work from {via}. Host aliases from ~/.ssh/config on {via} are \
                         not used."
                    ),
                    Reason::AdministrativelyProhibited => write!(
                        f,
                        "{via} does not allow connections through it to {address}:{port}. \
                         Check AllowTcpForwarding and PermitOpen in the SSH server settings \
                         on {via}."
                    ),
                    _ => write!(
                        f,
                        "{via} could not open a connection to {hop} at {address}:{port} \
                         ({reason:?})."
                    ),
                }
            }
            Self::Forward(reason) | Self::Sftp(reason) => write!(f, "{reason}"),
            Self::Cancelled => write!(f, "Cancelled."),
        }
    }
}

impl std::error::Error for SshError {}

impl From<russh::Error> for SshError {
    fn from(err: russh::Error) -> Self {
        Self::Protocol(err)
    }
}

impl From<io::Error> for SshError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// A question the user has to answer while connecting.
pub enum Prompt {
    HostKey {
        hop: String,
        address: String,
        port: u16,
        algorithm: String,
        fingerprint: String,
        status: HostKeyStatus,
        reply: oneshot::Sender<bool>,
    },
    Password {
        hop: String,
        username: String,
        retry: bool,
        /// Whether the answer can be saved in the system credential store.
        can_remember: bool,
        reply: oneshot::Sender<Option<Secret>>,
    },
    Passphrase {
        key_path: String,
        retry: bool,
        can_remember: bool,
        reply: oneshot::Sender<Option<Secret>>,
    },
}

/// A password or passphrase typed by the user.
pub struct Secret {
    pub value: String,
    /// The user asked to save it for next time.
    pub remember: bool,
}

/// Sends a `Prompt` to the UI and waits for the answer.
#[derive(Clone)]
pub struct Prompter {
    emit: Arc<dyn Fn(Prompt) + Send + Sync>,
    secrets: SecretStore,
}

impl Prompter {
    pub fn new(emit: impl Fn(Prompt) + Send + Sync + 'static, secrets: SecretStore) -> Self {
        Self {
            emit: Arc::new(emit),
            secrets,
        }
    }

    /// The saved secret under `key`, if any. The credential store can block,
    /// so it is read off the async threads.
    async fn saved(&self, key: &str) -> Option<String> {
        if !self.secrets.available() {
            return None;
        }
        let (secrets, key) = (self.secrets.clone(), key.to_string());
        tokio::task::spawn_blocking(move || secrets.get(&key))
            .await
            .ok()
            .flatten()
    }

    /// Save a secret that just worked. Failing to save does not fail the
    /// connection; the user is simply asked again next time.
    async fn remember(&self, key: &str, label: &str, secret: &str) {
        let secrets = self.secrets.clone();
        let (key, label, secret) = (key.to_string(), label.to_string(), secret.to_string());
        let _ = tokio::task::spawn_blocking(move || secrets.save(&key, &label, &secret)).await;
    }

    async fn forget(&self, key: &str) {
        let (secrets, key) = (self.secrets.clone(), key.to_string());
        let _ = tokio::task::spawn_blocking(move || secrets.forget(&key)).await;
    }

    /// `None` if the prompt was dropped without an answer.
    async fn ask<T>(&self, make: impl FnOnce(oneshot::Sender<T>) -> Prompt) -> Option<T> {
        let (tx, rx) = oneshot::channel();
        (self.emit)(make(tx));
        rx.await.ok()
    }
}

/// Channels the server opens towards us for a remote port forward.
pub type ForwardedChannels = mpsc::UnboundedSender<russh::Channel<client::Msg>>;

pub struct ClientHandler {
    hop: Hop,
    known_hosts: KnownHosts,
    prompter: Prompter,
    /// Set only on a connection that asked for a remote port forward.
    forwarded: Option<ForwardedChannels>,
}

impl client::Handler for ClientHandler {
    type Error = SshError;

    async fn check_server_key(
        &mut self,
        server_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        use russh::keys::PublicKeyOrCertificate as Presented;
        // A host certificate is judged by the public key inside it; host CA
        // validation is not supported yet, so it is treated as a plain key.
        let key = match server_key {
            Presented::PublicKey { key, .. } => key.clone(),
            Presented::Certificate(cert) => PublicKey::new(cert.public_key().clone(), ""),
        };
        let status = self
            .known_hosts
            .check(&self.hop.address, self.hop.port, &key)?;
        if status == HostKeyStatus::Trusted {
            return Ok(true);
        }
        let accepted = self
            .prompter
            .ask(|reply| Prompt::HostKey {
                hop: self.hop.name.clone(),
                address: self.hop.address.clone(),
                port: self.hop.port,
                algorithm: key.algorithm().to_string(),
                fingerprint: known_hosts::fingerprint(&key),
                status,
                reply,
            })
            .await
            .unwrap_or(false);
        if !accepted {
            return Err(SshError::HostKeyRejected {
                hop: self.hop.name.clone(),
            });
        }
        self.known_hosts
            .trust(&self.hop.address, self.hop.port, &key)?;
        Ok(true)
    }

    /// The server wants to open a connection back to us. That is accepted
    /// only on a connection where the user set up a remote port forward;
    /// everywhere else the reply is dropped, which rejects the channel.
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if let Some(forwarded) = &self.forwarded
            && forwarded.send(channel).is_ok()
        {
            reply.accept().await;
        }
        Ok(())
    }
}

/// An authenticated connection to the target host. The connection to each
/// jump host is kept alongside it because the target is tunnelled through them.
pub struct Connection {
    /// Outermost jump host first, target last.
    handles: Vec<Handle<ClientHandler>>,
}

impl Connection {
    pub fn target(&self) -> &Handle<ClientHandler> {
        self.handles.last().expect("at least one hop")
    }

    pub async fn close(self) {
        for handle in self.handles.into_iter().rev() {
            let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
        }
    }
}

/// Connect through each hop in order, authenticating at every one.
///
/// The first hop is reached directly over TCP. Each following hop is reached
/// through a `direct-tcpip` channel on the previous hop, so traffic to the
/// target is encrypted end to end and jump hosts cannot read it.
pub async fn connect(
    hops: &[Hop],
    known_hosts: &KnownHosts,
    prompter: &Prompter,
    status: &(dyn Fn(String) + Send + Sync),
) -> Result<Connection, SshError> {
    connect_with(hops, known_hosts, prompter, status, None).await
}

/// Like [`connect`], and when `forwarded` is given the target hop accepts
/// channels the server opens for a remote port forward and sends them there.
pub async fn connect_with(
    hops: &[Hop],
    known_hosts: &KnownHosts,
    prompter: &Prompter,
    status: &(dyn Fn(String) + Send + Sync),
    forwarded: Option<ForwardedChannels>,
) -> Result<Connection, SshError> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        ..Default::default()
    });
    let mut handles: Vec<Handle<ClientHandler>> = Vec::new();

    for (index, hop) in hops.iter().enumerate() {
        if hop.username.is_empty() {
            return Err(SshError::MissingUsername {
                hop: hop.name.clone(),
            });
        }
        status(format!("Connecting to {}", hop.name));
        let handler = ClientHandler {
            hop: hop.clone(),
            known_hosts: known_hosts.clone(),
            prompter: prompter.clone(),
            // Only the target may open channels back to us.
            forwarded: forwarded.clone().filter(|_| index + 1 == hops.len()),
        };
        let timeout = || SshError::Timeout {
            hop: hop.name.clone(),
        };
        let mut handle = match handles.last() {
            None => {
                tokio::time::timeout(
                    CONNECT_TIMEOUT,
                    tokio::net::TcpStream::connect((hop.address.as_str(), hop.port)),
                )
                .await
                .map_err(|_| timeout())
                .and_then(|stream| stream.map_err(SshError::from))
                .map(|stream| client::connect_stream(config.clone(), stream, handler))?
                .await?
            }
            Some(previous) => {
                let channel = tokio::time::timeout(
                    CONNECT_TIMEOUT,
                    previous.channel_open_direct_tcpip(
                        hop.address.clone(),
                        hop.port as u32,
                        "127.0.0.1",
                        0,
                    ),
                )
                .await
                .map_err(|_| timeout())?
                .map_err(|err| match err {
                    russh::Error::ChannelOpenFailure(reason) => SshError::JumpRefused {
                        via: hops[index - 1].name.clone(),
                        hop: hop.name.clone(),
                        address: hop.address.clone(),
                        port: hop.port,
                        reason,
                    },
                    err => err.into(),
                })?;
                client::connect_stream(config.clone(), channel.into_stream(), handler).await?
            }
        };
        status(format!("Authenticating to {}", hop.name));
        authenticate(&mut handle, hop, prompter).await?;
        handles.push(handle);
    }
    Ok(Connection { handles })
}

async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    hop: &Hop,
    prompter: &Prompter,
) -> Result<(), SshError> {
    match &hop.auth {
        HopAuth::Password => {
            // The host key was verified before this point, so a saved
            // password is only ever sent to the host it was saved for.
            let key = secrets::password_key(&hop.username, &hop.address, hop.port);
            let mut saved_rejected = false;
            if let Some(saved) = prompter.saved(&key).await {
                let result = handle
                    .authenticate_password(hop.username.clone(), saved)
                    .await?;
                if result.success() {
                    return Ok(());
                }
                // It no longer works; drop it and ask.
                prompter.forget(&key).await;
                saved_rejected = true;
            }
            for attempt in 0..MAX_SECRET_ATTEMPTS {
                let answer = prompter
                    .ask(|reply| Prompt::Password {
                        hop: hop.name.clone(),
                        username: hop.username.clone(),
                        retry: attempt > 0 || saved_rejected,
                        can_remember: prompter.secrets.available(),
                        reply,
                    })
                    .await
                    .flatten()
                    .ok_or(SshError::Cancelled)?;
                let result = handle
                    .authenticate_password(hop.username.clone(), answer.value.clone())
                    .await?;
                if result.success() {
                    if answer.remember {
                        let label =
                            format!("Password for {}@{}:{}", hop.username, hop.address, hop.port);
                        prompter.remember(&key, &label, &answer.value).await;
                    }
                    return Ok(());
                }
            }
        }
        HopAuth::Key {
            path,
            name,
            certificate,
        } => {
            let key = Arc::new(load_key(path, name, prompter).await?);
            if let Some(certificate) = certificate {
                let certificate =
                    Certificate::from_openssh(certificate).map_err(|err| SshError::Key {
                        path: name.clone(),
                        reason: format!("its certificate cannot be read ({err})"),
                    })?;
                let result = handle
                    .authenticate_openssh_cert(hop.username.clone(), key.clone(), certificate)
                    .await?;
                if result.success() {
                    return Ok(());
                }
                // The server may still accept the plain key.
            }
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            let result = handle
                .authenticate_publickey(hop.username.clone(), PrivateKeyWithHashAlg::new(key, hash))
                .await?;
            if result.success() {
                return Ok(());
            }
        }
    }
    Err(SshError::AuthFailed {
        hop: hop.name.clone(),
    })
}

/// Load a private key, asking for a passphrase only if the key is encrypted.
async fn load_key(
    path: &str,
    name: &str,
    prompter: &Prompter,
) -> Result<russh::keys::PrivateKey, SshError> {
    let resolved = expand_home(path);
    let key_error = |reason: String| SshError::Key {
        path: name.to_string(),
        reason,
    };
    match load_secret_key(&resolved, None) {
        Ok(key) => return Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => {}
        Err(err) => return Err(key_error(err.to_string())),
    }
    let key = secrets::passphrase_key(&resolved.display().to_string());
    let mut saved_rejected = false;
    if let Some(saved) = prompter.saved(&key).await {
        if let Ok(private) = load_secret_key(&resolved, Some(&saved)) {
            return Ok(private);
        }
        prompter.forget(&key).await;
        saved_rejected = true;
    }
    for attempt in 0..MAX_SECRET_ATTEMPTS {
        let answer = prompter
            .ask(|reply| Prompt::Passphrase {
                key_path: name.to_string(),
                retry: attempt > 0 || saved_rejected,
                can_remember: prompter.secrets.available(),
                reply,
            })
            .await
            .flatten()
            .ok_or(SshError::Cancelled)?;
        if let Ok(private) = load_secret_key(&resolved, Some(&answer.value)) {
            if answer.remember {
                let label = format!("Passphrase for key {name}");
                prompter.remember(&key, &label, &answer.value).await;
            }
            return Ok(private);
        }
    }
    Err(key_error("wrong passphrase".to_string()))
}

/// Expand a leading `~/` to the user's home folder.
pub fn expand_home(path: &str) -> PathBuf {
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    match (rest, directories::BaseDirs::new()) {
        (Some(rest), Some(dirs)) => dirs.home_dir().join(rest),
        _ => PathBuf::from(path),
    }
}

/// Events from a shell session to the UI.
pub enum SessionEvent {
    Status(String),
    Prompt(Prompt),
    Connected,
    Output(Vec<u8>),
    /// The session ended. `error` is `None` if it closed normally.
    Closed {
        error: Option<String>,
    },
}

pub enum SessionCommand {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

/// Handle to a shell session running in the background. Dropping it closes
/// the connection.
pub struct SessionHandle {
    pub events: std::sync::mpsc::Receiver<SessionEvent>,
    commands: mpsc::UnboundedSender<SessionCommand>,
}

impl SessionHandle {
    pub fn send(&self, command: SessionCommand) {
        let _ = self.commands.send(command);
    }
}

/// Start an interactive shell session through `hops`. `wake` is called on
/// every new event so the UI can repaint.
pub fn spawn_shell(
    runtime: &tokio::runtime::Handle,
    hops: Vec<Hop>,
    known_hosts: KnownHosts,
    secrets: SecretStore,
    (cols, rows): (u16, u16),
    wake: impl Fn() + Send + Sync + 'static,
) -> SessionHandle {
    let (event_tx, events) = std::sync::mpsc::channel();
    let (commands, mut command_rx) = mpsc::unbounded_channel();
    let emit: Arc<dyn Fn(SessionEvent) + Send + Sync> = Arc::new(move |event| {
        let _ = event_tx.send(event);
        wake();
    });

    runtime.spawn(async move {
        let prompter = {
            let emit = emit.clone();
            Prompter::new(move |prompt| emit(SessionEvent::Prompt(prompt)), secrets)
        };
        let status = {
            let emit = emit.clone();
            move |text| emit(SessionEvent::Status(text))
        };
        let result = async {
            // Stop connecting if the tab is closed while waiting.
            let connection = tokio::select! {
                connection = connect(&hops, &known_hosts, &prompter, &status) => connection?,
                _ = commands_closed(&mut command_rx) => return Err(SshError::Cancelled),
            };
            let outcome = run_shell(&connection, (cols, rows), &mut command_rx, &emit).await;
            connection.close().await;
            outcome
        }
        .await;
        emit(SessionEvent::Closed {
            error: match result {
                Ok(()) | Err(SshError::Cancelled) => None,
                Err(err) => Some(err.to_string()),
            },
        });
    });

    SessionHandle { events, commands }
}

/// Resolves when the `SessionHandle` is dropped. Commands that arrive before
/// the connection is ready are discarded; the UI sends the size again as soon
/// as the session is connected.
async fn commands_closed(commands: &mut mpsc::UnboundedReceiver<SessionCommand>) {
    while commands.recv().await.is_some() {}
}

async fn run_shell(
    connection: &Connection,
    (cols, rows): (u16, u16),
    commands: &mut mpsc::UnboundedReceiver<SessionCommand>,
    emit: &Arc<dyn Fn(SessionEvent) + Send + Sync>,
) -> Result<(), SshError> {
    let mut channel = connection.target().channel_open_session().await?;
    channel
        .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await?;
    channel.request_shell(true).await?;
    emit(SessionEvent::Connected);

    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(SessionCommand::Input(bytes)) => channel.data_bytes(bytes).await?,
                Some(SessionCommand::Resize { cols, rows }) => {
                    channel.window_change(cols as u32, rows as u32, 0, 0).await?;
                }
                None => return Ok(()),
            },
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) => emit(SessionEvent::Output(data.to_vec())),
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    emit(SessionEvent::Output(data.to_vec()));
                }
                Some(ChannelMsg::Close) | None => return Ok(()),
                Some(_) => {}
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
