//! Copies a file or a whole folder between any two places: this computer
//! and a server, in either direction, or two servers. Server-to-server
//! copies stream through this computer; nothing is stored on it.

use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use russh_sftp::client::SftpSession;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{CHUNK, EntryKind, PROGRESS_INTERVAL, join, partial_path};

/// A connected SFTP session that copies can read from or write to. Get it
/// from [`super::SftpHandle::peer`]; it is empty until the session connects.
#[derive(Clone, Default)]
pub struct Peer(Arc<OnceLock<Arc<SftpSession>>>);

impl Peer {
    pub(super) fn set(&self, session: Arc<SftpSession>) {
        let _ = self.0.set(session);
    }

    fn session(&self) -> Result<Arc<SftpSession>, String> {
        self.0
            .get()
            .cloned()
            .ok_or_else(|| "The server is not connected yet.".to_string())
    }
}

/// A file or folder on this computer or on a server.
#[derive(Clone)]
pub enum Place {
    Local(PathBuf),
    Remote { peer: Peer, path: String },
}

pub enum CopyEvent {
    /// Bytes copied so far, out of the total found before copying started.
    Progress { done: u64, total: u64 },
    /// The copy ended. `error` is `None` on success. `skipped` counts links
    /// and special files inside a folder that were left out.
    Done {
        error: Option<String>,
        skipped: usize,
    },
}

/// A running copy. Dropping it does not stop the copy; `cancel` does.
pub struct CopyHandle {
    pub events: std::sync::mpsc::Receiver<CopyEvent>,
    cancel: Arc<AtomicBool>,
}

impl CopyHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Copy `from` to `to` in the background. `to` is the full destination,
/// including the new name. A folder is copied with everything in it and
/// merged into a folder that already exists; existing files are replaced.
pub fn spawn_copy(
    runtime: &tokio::runtime::Handle,
    from: Place,
    to: Place,
    wake: impl Fn() + Send + Sync + 'static,
) -> CopyHandle {
    let (event_tx, events) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let emit = move |event| {
        let _ = event_tx.send(event);
        wake();
    };
    let cancelled = cancel.clone();
    runtime.spawn(async move {
        let mut skipped = 0;
        let error = copy(from, to, &cancelled, &mut skipped, &emit).await.err();
        emit(CopyEvent::Done { error, skipped });
    });
    CopyHandle { events, cancel }
}

/// One file or folder, resolved to where it lives.
#[derive(Clone)]
enum Node {
    Local(PathBuf),
    Remote(Arc<SftpSession>, String),
}

type Reader = Box<dyn AsyncRead + Send + Unpin>;
type Writer = Box<dyn AsyncWrite + Send + Unpin>;

fn text(err: impl std::fmt::Display) -> String {
    err.to_string()
}

impl Node {
    fn resolve(place: Place) -> Result<Self, String> {
        Ok(match place {
            Place::Local(path) => Self::Local(path),
            Place::Remote { peer, path } => Self::Remote(peer.session()?, path),
        })
    }

    fn child(&self, name: &str) -> Self {
        match self {
            Self::Local(path) => Self::Local(path.join(name)),
            Self::Remote(sftp, path) => Self::Remote(sftp.clone(), join(path, name)),
        }
    }

    fn name(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Remote(_, path) => path.clone(),
        }
    }

    fn same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local(a), Self::Local(b)) => a == b,
            (Self::Remote(a, path_a), Self::Remote(b, path_b)) => {
                Arc::ptr_eq(a, b) && path_a.trim_end_matches('/') == path_b.trim_end_matches('/')
            }
            _ => false,
        }
    }

    /// Kind and size, following links. `None` when nothing is there.
    async fn stat(&self) -> Option<(EntryKind, u64)> {
        match self {
            Self::Local(path) => {
                let meta = tokio::fs::metadata(path).await.ok()?;
                let kind = if meta.is_dir() {
                    EntryKind::Dir
                } else if meta.is_file() {
                    EntryKind::File
                } else {
                    EntryKind::Other
                };
                Some((kind, meta.len()))
            }
            Self::Remote(sftp, path) => {
                let meta = sftp.metadata(path.clone()).await.ok()?;
                let kind = if meta.is_dir() {
                    EntryKind::Dir
                } else if meta.is_regular() {
                    EntryKind::File
                } else {
                    EntryKind::Other
                };
                Some((kind, meta.size.unwrap_or(0)))
            }
        }
    }

    /// Name, kind and size of everything in this folder. Links are reported
    /// as links, not as what they point to.
    async fn children(&self) -> Result<Vec<(String, EntryKind, u64)>, String> {
        let mut children = Vec::new();
        match self {
            Self::Local(path) => {
                let mut dir = tokio::fs::read_dir(path).await.map_err(text)?;
                while let Some(entry) = dir.next_entry().await.map_err(text)? {
                    let kind = entry.file_type().await.map_err(text)?;
                    let kind = if kind.is_symlink() {
                        EntryKind::Symlink
                    } else if kind.is_dir() {
                        EntryKind::Dir
                    } else if kind.is_file() {
                        EntryKind::File
                    } else {
                        EntryKind::Other
                    };
                    let size = entry.metadata().await.map(|meta| meta.len()).unwrap_or(0);
                    children.push((entry.file_name().to_string_lossy().to_string(), kind, size));
                }
            }
            Self::Remote(sftp, path) => {
                for entry in sftp.read_dir(path.clone()).await.map_err(text)? {
                    let name = entry.file_name();
                    if name == "." || name == ".." {
                        continue;
                    }
                    let meta = entry.metadata();
                    let kind = if meta.is_symlink() {
                        EntryKind::Symlink
                    } else if meta.is_dir() {
                        EntryKind::Dir
                    } else if meta.is_regular() {
                        EntryKind::File
                    } else {
                        EntryKind::Other
                    };
                    children.push((name, kind, meta.size.unwrap_or(0)));
                }
            }
        }
        children.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(children)
    }

    /// Create this folder unless it is already there.
    async fn make_dir(&self) -> Result<(), String> {
        match self.stat().await {
            Some((EntryKind::Dir, _)) => return Ok(()),
            Some(_) => return Err(format!("{} exists and is not a folder", self.name())),
            None => {}
        }
        match self {
            Self::Local(path) => tokio::fs::create_dir(path).await.map_err(text),
            Self::Remote(sftp, path) => sftp.create_dir(path.clone()).await.map_err(text),
        }
        .map_err(|err| format!("Could not create {}: {err}", self.name()))
    }

    async fn reader(&self) -> Result<Reader, String> {
        Ok(match self {
            Self::Local(path) => Box::new(tokio::fs::File::open(path).await.map_err(text)?),
            Self::Remote(sftp, path) => Box::new(sftp.open(path.clone()).await.map_err(text)?),
        })
    }

    /// Where the bytes are written while a copy runs: a `.part` file next to
    /// a local target, so a failed copy never leaves a cut-off file under
    /// the real name; the target itself on a server.
    fn staging(&self) -> Self {
        match self {
            Self::Local(path) => Self::Local(partial_path(path)),
            Self::Remote(..) => self.clone(),
        }
    }

    async fn writer(&self) -> Result<Writer, String> {
        Ok(match self {
            Self::Local(path) => Box::new(tokio::fs::File::create(path).await.map_err(text)?),
            Self::Remote(sftp, path) => Box::new(sftp.create(path.clone()).await.map_err(text)?),
        })
    }

    async fn remove_file(&self) {
        let _ = match self {
            Self::Local(path) => tokio::fs::remove_file(path).await.map_err(text),
            Self::Remote(sftp, path) => sftp.remove_file(path.clone()).await.map_err(text),
        };
    }
}

/// Reports progress at most every `PROGRESS_INTERVAL`, and always at the end.
struct Progress<'a> {
    done: u64,
    total: u64,
    last: Instant,
    emit: &'a (dyn Fn(CopyEvent) + Send + Sync),
}

impl Progress<'_> {
    fn add(&mut self, bytes: usize) {
        self.done += bytes as u64;
        if self.last.elapsed() >= PROGRESS_INTERVAL || self.done >= self.total {
            self.report();
        }
    }

    fn report(&mut self) {
        self.last = Instant::now();
        (self.emit)(CopyEvent::Progress {
            done: self.done,
            total: self.total.max(self.done),
        });
    }
}

const CANCELLED: &str = "Cancelled";

async fn copy(
    from: Place,
    to: Place,
    cancel: &AtomicBool,
    skipped: &mut usize,
    emit: &(dyn Fn(CopyEvent) + Send + Sync),
) -> Result<(), String> {
    let from = Node::resolve(from)?;
    let to = Node::resolve(to)?;
    if from.same_as(&to) {
        return Err("The source and the destination are the same.".to_string());
    }

    // Find everything first, so progress has a total and a folder copied
    // into itself cannot grow forever.
    let mut folders: Vec<Node> = Vec::new();
    let mut files: Vec<(Node, Node, u64)> = Vec::new();
    match from.stat().await {
        Some((EntryKind::File, size)) => files.push((from, to, size)),
        Some((EntryKind::Dir, _)) => {
            let mut pending = vec![(from, to)];
            while let Some((source, target)) = pending.pop() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(CANCELLED.to_string());
                }
                let children = source
                    .children()
                    .await
                    .map_err(|err| format!("Could not read {}: {err}", source.name()))?;
                folders.push(target.clone());
                for (name, kind, size) in children {
                    match kind {
                        EntryKind::Dir => pending.push((source.child(&name), target.child(&name))),
                        EntryKind::File => {
                            files.push((source.child(&name), target.child(&name), size));
                        }
                        EntryKind::Symlink | EntryKind::Other => *skipped += 1,
                    }
                }
            }
        }
        Some(_) => return Err("Only files and folders can be transferred.".to_string()),
        None => return Err(format!("{} could not be read.", from.name())),
    }

    let mut progress = Progress {
        done: 0,
        total: files.iter().map(|(_, _, size)| size).sum(),
        last: Instant::now(),
        emit,
    };
    progress.report();
    for folder in &folders {
        folder.make_dir().await?;
    }
    for (source, target, _) in &files {
        copy_file(source, target, cancel, &mut progress)
            .await
            .map_err(|err| {
                if folders.is_empty() || err == CANCELLED {
                    err
                } else {
                    format!("{}: {err}", source.name())
                }
            })?;
    }
    progress.report();
    Ok(())
}

async fn copy_file(
    source: &Node,
    target: &Node,
    cancel: &AtomicBool,
    progress: &mut Progress<'_>,
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err(CANCELLED.to_string());
    }
    let mut reader = source.reader().await?;
    let staging = target.staging();
    let mut writer = staging.writer().await?;
    let result = async {
        let mut buffer = vec![0u8; CHUNK];
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(CANCELLED.to_string());
            }
            let read = reader.read(&mut buffer).await.map_err(text)?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read]).await.map_err(text)?;
            progress.add(read);
        }
        writer.shutdown().await.map_err(text)
    }
    .await;
    drop(writer);
    match (result, &staging, target) {
        (Ok(()), Node::Local(partial), Node::Local(path)) => {
            tokio::fs::rename(partial, path).await.map_err(|err| {
                let _ = std::fs::remove_file(partial);
                err.to_string()
            })
        }
        (Ok(()), ..) => Ok(()),
        (Err(err), ..) => {
            staging.remove_file().await;
            Err(err)
        }
    }
}
