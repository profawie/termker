//! SFTP: a background session that lists directories, manages files and
//! transfers them, built on `russh-sftp` over the regular SSH connection.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use russh_sftp::client::SftpSession;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

use termker_core::secrets::SecretStore;

use crate::{self as ssh, Hop, KnownHosts, Prompt, Prompter, SshError};

const CHUNK: usize = 64 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits, when the server reports them.
    pub permissions: Option<u32>,
}

/// Join a remote directory and a name with exactly one `/` between them.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Parent of a remote path, or `None` at the root.
pub fn parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(index) => Some(trimmed[..index].to_string()),
        None => None,
    }
}

/// A name is safe to use as a single path component.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

/// Folders first, then by name without regard to case.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        let dir = |entry: &Entry| entry.kind != EntryKind::Dir;
        dir(a)
            .cmp(&dir(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Human-readable size, e.g. `1.5 MB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `rwxr-xr-x` style text for Unix permission bits.
pub fn format_permissions(mode: u32) -> String {
    (0..9)
        .map(|bit| {
            if mode & (0o400 >> bit) != 0 {
                ['r', 'w', 'x'][bit % 3]
            } else {
                '-'
            }
        })
        .collect()
}

pub use termker_core::time::format_time;

pub enum Command {
    List(String),
    CreateDir(String),
    Remove { path: String, is_dir: bool },
    Rename { from: String, to: String },
    Download(Transfer),
    Upload(Transfer),
}

/// One file to copy. `cancel` can be set at any time to stop it.
pub struct Transfer {
    pub id: u64,
    pub remote: String,
    pub local: PathBuf,
    pub cancel: Arc<AtomicBool>,
}

pub enum Event {
    Status(String),
    Prompt(Prompt),
    /// Connected; `home` is the directory the server starts in.
    Connected {
        home: String,
    },
    Listing {
        path: String,
        entries: Vec<Entry>,
    },
    /// A list, create, delete or rename request failed.
    Failed(String),
    Progress {
        id: u64,
        done: u64,
        total: u64,
    },
    /// A transfer ended. `error` is `None` on success.
    TransferDone {
        id: u64,
        error: Option<String>,
    },
    Closed {
        error: Option<String>,
    },
}

/// Handle to a background SFTP session. Dropping it disconnects.
pub struct SftpHandle {
    pub events: std::sync::mpsc::Receiver<Event>,
    commands: mpsc::UnboundedSender<Command>,
}

impl SftpHandle {
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

type Emit = Arc<dyn Fn(Event) + Send + Sync>;

pub fn spawn_sftp(
    runtime: &tokio::runtime::Handle,
    hops: Vec<Hop>,
    known_hosts: KnownHosts,
    secrets: SecretStore,
    wake: impl Fn() + Send + Sync + 'static,
) -> SftpHandle {
    let (event_tx, events) = std::sync::mpsc::channel();
    let (commands, mut command_rx) = mpsc::unbounded_channel();
    let emit: Emit = Arc::new(move |event| {
        let _ = event_tx.send(event);
        wake();
    });

    runtime.spawn(async move {
        let result = run(&hops, &known_hosts, secrets, &mut command_rx, &emit).await;
        emit(Event::Closed {
            error: match result {
                Ok(()) | Err(SshError::Cancelled) => None,
                Err(err) => Some(err.to_string()),
            },
        });
    });

    SftpHandle { events, commands }
}

fn sftp_error(action: &str, err: impl std::fmt::Display) -> SshError {
    SshError::Sftp(format!("{action}: {err}"))
}

async fn run(
    hops: &[Hop],
    known_hosts: &KnownHosts,
    secrets: SecretStore,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    emit: &Emit,
) -> Result<(), SshError> {
    let prompter = {
        let emit = emit.clone();
        Prompter::new(move |prompt| emit(Event::Prompt(prompt)), secrets)
    };
    let status = {
        let emit = emit.clone();
        move |text| emit(Event::Status(text))
    };
    // Stop connecting if the handle is dropped while waiting.
    let connection = tokio::select! {
        connection = ssh::connect(hops, known_hosts, &prompter, &status) => connection?,
        _ = async { while commands.recv().await.is_some() {} } => return Err(SshError::Cancelled),
    };
    let channel = connection.target().channel_open_session().await?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|_| SshError::Sftp("This server does not offer SFTP.".to_string()))?;
    let sftp = Arc::new(
        SftpSession::new(channel.into_stream())
            .await
            .map_err(|err| sftp_error("Could not start SFTP", err))?,
    );
    let home = sftp
        .canonicalize(".")
        .await
        .unwrap_or_else(|_| "/".to_string());
    emit(Event::Connected { home: home.clone() });
    list(&sftp, &home, emit).await;

    loop {
        let command = tokio::select! {
            command = commands.recv() => command,
            _ = lost(&connection) => {
                return Err(SshError::Sftp("The SSH connection was lost.".to_string()));
            }
        };
        let Some(command) = command else {
            let _ = sftp.close().await;
            return Ok(());
        };
        match command {
            Command::List(path) => list(&sftp, &path, emit).await,
            Command::CreateDir(path) => {
                let result = sftp.create_dir(path.clone()).await;
                finish(&sftp, emit, "Could not create the folder", result, &path).await;
            }
            Command::Remove { path, is_dir } => {
                let result = if is_dir {
                    sftp.remove_dir(path.clone()).await
                } else {
                    sftp.remove_file(path.clone()).await
                };
                finish(&sftp, emit, "Could not delete", result, &path).await;
            }
            Command::Rename { from, to } => {
                let result = sftp.rename(from, to.clone()).await;
                finish(&sftp, emit, "Could not rename", result, &to).await;
            }
            Command::Download(transfer) => {
                let (sftp, emit) = (sftp.clone(), emit.clone());
                tokio::spawn(async move {
                    let error = download(&sftp, &transfer, &emit).await.err();
                    emit(Event::TransferDone {
                        id: transfer.id,
                        error,
                    });
                });
            }
            Command::Upload(transfer) => {
                let (sftp, emit) = (sftp.clone(), emit.clone());
                tokio::spawn(async move {
                    let error = upload(&sftp, &transfer, &emit).await.err();
                    emit(Event::TransferDone {
                        id: transfer.id,
                        error,
                    });
                    // Show the new file without a manual refresh.
                    if let Some(dir) = parent(&transfer.remote) {
                        list(&sftp, &dir, &emit).await;
                    }
                });
            }
        }
    }
}

/// Resolves when the SSH connection is gone.
async fn lost(connection: &ssh::Connection) {
    while !connection.target().is_closed() {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Report a failed change, or list the directory the change happened in.
async fn finish(
    sftp: &SftpSession,
    emit: &Emit,
    action: &str,
    result: Result<(), russh_sftp::client::error::Error>,
    changed_path: &str,
) {
    match result {
        Ok(()) => {
            if let Some(dir) = parent(changed_path) {
                list(sftp, &dir, emit).await;
            }
        }
        Err(err) => emit(Event::Failed(format!("{action}: {err}"))),
    }
}

async fn list(sftp: &SftpSession, path: &str, emit: &Emit) {
    match sftp.read_dir(path).await {
        Ok(dir) => {
            let mut entries: Vec<Entry> = dir
                .filter(|entry| !matches!(entry.file_name().as_str(), "." | ".."))
                .map(|entry| {
                    let meta = entry.metadata();
                    Entry {
                        name: entry.file_name(),
                        kind: if meta.is_dir() {
                            EntryKind::Dir
                        } else if meta.is_symlink() {
                            EntryKind::Symlink
                        } else if meta.is_regular() {
                            EntryKind::File
                        } else {
                            EntryKind::Other
                        },
                        size: meta.size.unwrap_or(0),
                        modified: meta.modified().ok(),
                        permissions: meta.permissions.map(|mode| mode & 0o777),
                    }
                })
                .collect();
            sort_entries(&mut entries);
            emit(Event::Listing {
                path: path.to_string(),
                entries,
            });
        }
        Err(err) => emit(Event::Failed(format!("Could not open {path}: {err}"))),
    }
}

/// Reports progress at most every `PROGRESS_INTERVAL`, and always at the end.
struct Progress<'a> {
    id: u64,
    total: u64,
    done: u64,
    last: Instant,
    emit: &'a Emit,
}

impl<'a> Progress<'a> {
    fn new(id: u64, total: u64, emit: &'a Emit) -> Self {
        emit(Event::Progress { id, done: 0, total });
        Self {
            id,
            total,
            done: 0,
            last: Instant::now(),
            emit,
        }
    }

    fn add(&mut self, bytes: usize) {
        self.done += bytes as u64;
        if self.last.elapsed() >= PROGRESS_INTERVAL || self.done >= self.total {
            self.last = Instant::now();
            (self.emit)(Event::Progress {
                id: self.id,
                done: self.done,
                total: self.total.max(self.done),
            });
        }
    }
}

/// Path of the temporary file a download is written to first.
pub fn partial_path(local: &Path) -> PathBuf {
    let mut name = local.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    local.with_file_name(name)
}

/// Download to `<name>.part` and rename when complete, so a failed or
/// cancelled download never leaves a truncated file under the real name.
async fn download(sftp: &SftpSession, transfer: &Transfer, emit: &Emit) -> Result<(), String> {
    let partial = partial_path(&transfer.local);
    let result = async {
        let mut remote = sftp
            .open(transfer.remote.clone())
            .await
            .map_err(|err| err.to_string())?;
        let total = remote.metadata().await.map(|meta| meta.len()).unwrap_or(0);
        let mut local = tokio::fs::File::create(&partial)
            .await
            .map_err(|err| err.to_string())?;
        let mut progress = Progress::new(transfer.id, total, emit);
        let mut buffer = vec![0u8; CHUNK];
        loop {
            if transfer.cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let read = remote
                .read(&mut buffer)
                .await
                .map_err(|err| err.to_string())?;
            if read == 0 {
                break;
            }
            local
                .write_all(&buffer[..read])
                .await
                .map_err(|err| err.to_string())?;
            progress.add(read);
        }
        local.flush().await.map_err(|err| err.to_string())?;
        drop(local);
        tokio::fs::rename(&partial, &transfer.local)
            .await
            .map_err(|err| err.to_string())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&partial).await;
    }
    result
}

/// Upload, removing the incomplete remote file if it fails or is cancelled.
async fn upload(sftp: &SftpSession, transfer: &Transfer, emit: &Emit) -> Result<(), String> {
    let mut local = tokio::fs::File::open(&transfer.local)
        .await
        .map_err(|err| err.to_string())?;
    let total = local.metadata().await.map(|meta| meta.len()).unwrap_or(0);
    let mut remote = sftp
        .create(transfer.remote.clone())
        .await
        .map_err(|err| err.to_string())?;
    let result = async {
        let mut progress = Progress::new(transfer.id, total, emit);
        let mut buffer = vec![0u8; CHUNK];
        loop {
            if transfer.cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let read = local
                .read(&mut buffer)
                .await
                .map_err(|err| err.to_string())?;
            if read == 0 {
                break;
            }
            remote
                .write_all(&buffer[..read])
                .await
                .map_err(|err| err.to_string())?;
            progress.add(read);
        }
        remote.shutdown().await.map_err(|err| err.to_string())
    }
    .await;
    if result.is_err() {
        drop(remote);
        let _ = sftp.remove_file(transfer.remote.clone()).await;
    }
    result
}

#[cfg(test)]
mod tests;
