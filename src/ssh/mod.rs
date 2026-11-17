//! SSH connections: host key verification, user authentication, jump hosts
//! (ProxyJump) and interactive shell sessions. Built on `russh`.

pub mod known_hosts;

use std::{fmt, io, path::PathBuf, sync::Arc, time::Duration};

use russh::{
    ChannelMsg, Disconnect,
    client::{self, Handle},
    keys::{PrivateKeyWithHashAlg, PublicKey, load_secret_key},
};
use tokio::sync::{mpsc, oneshot};

use crate::model::{AuthMethod, Host};
pub use known_hosts::{HostKeyStatus, KnownHosts};

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
    pub auth: AuthMethod,
}

impl From<&Host> for Hop {
    fn from(host: &Host) -> Self {
        Self {
            name: host.display_name().to_string(),
            address: host.address.clone(),
            port: host.port,
            username: host.username.clone(),
            auth: host.auth.clone(),
        }
    }
}

#[derive(Debug)]
pub enum SshError {
    Protocol(russh::Error),
    Io(io::Error),
    Timeout { hop: String },
    MissingUsername { hop: String },
    HostKeyRejected { hop: String },
    Key { path: String, reason: String },
    AuthFailed { hop: String },
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
        reply: oneshot::Sender<Option<String>>,
    },
    Passphrase {
        key_path: String,
        retry: bool,
        reply: oneshot::Sender<Option<String>>,
    },
}

/// Sends a `Prompt` to the UI and waits for the answer.
#[derive(Clone)]
pub struct Prompter {
    emit: Arc<dyn Fn(Prompt) + Send + Sync>,
}

impl Prompter {
    pub fn new(emit: impl Fn(Prompt) + Send + Sync + 'static) -> Self {
        Self {
            emit: Arc::new(emit),
        }
    }

    /// `None` if the prompt was dropped without an answer.
    async fn ask<T>(&self, make: impl FnOnce(oneshot::Sender<T>) -> Prompt) -> Option<T> {
        let (tx, rx) = oneshot::channel();
        (self.emit)(make(tx));
        rx.await.ok()
    }
}

pub struct ClientHandler {
    hop: Hop,
    known_hosts: KnownHosts,
    prompter: Prompter,
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
    let config = Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        ..Default::default()
    });
    let mut handles: Vec<Handle<ClientHandler>> = Vec::new();

    for hop in hops {
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
                .map_err(|_| timeout())??;
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
        AuthMethod::Password => {
            for attempt in 0..MAX_SECRET_ATTEMPTS {
                let password = prompter
                    .ask(|reply| Prompt::Password {
                        hop: hop.name.clone(),
                        username: hop.username.clone(),
                        retry: attempt > 0,
                        reply,
                    })
                    .await
                    .flatten()
                    .ok_or(SshError::Cancelled)?;
                let result = handle
                    .authenticate_password(hop.username.clone(), password)
                    .await?;
                if result.success() {
                    return Ok(());
                }
            }
        }
        AuthMethod::KeyFile { path } => {
            let key = load_key(path, prompter).await?;
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            let result = handle
                .authenticate_publickey(
                    hop.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
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
async fn load_key(path: &str, prompter: &Prompter) -> Result<russh::keys::PrivateKey, SshError> {
    let resolved = expand_home(path);
    let key_error = |reason: String| SshError::Key {
        path: path.to_string(),
        reason,
    };
    match load_secret_key(&resolved, None) {
        Ok(key) => return Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => {}
        Err(err) => return Err(key_error(err.to_string())),
    }
    for attempt in 0..MAX_SECRET_ATTEMPTS {
        let passphrase = prompter
            .ask(|reply| Prompt::Passphrase {
                key_path: path.to_string(),
                retry: attempt > 0,
                reply,
            })
            .await
            .flatten()
            .ok_or(SshError::Cancelled)?;
        if let Ok(key) = load_secret_key(&resolved, Some(&passphrase)) {
            return Ok(key);
        }
    }
    Err(key_error("wrong passphrase".to_string()))
}

/// Expand a leading `~/` to the user's home folder.
fn expand_home(path: &str) -> PathBuf {
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
            Prompter::new(move |prompt| emit(SessionEvent::Prompt(prompt)))
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
mod tests;
