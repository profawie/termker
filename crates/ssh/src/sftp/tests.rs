//! End-to-end SFTP tests against an in-process server that serves a
//! temporary directory.

use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    net::SocketAddr,
    path::PathBuf,
};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server::{self, Auth, Msg, Server as _, Session},
};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use tokio::net::TcpListener;

use super::*;
use crate::tests::{Answers, PASSWORD, hop, known_hosts, temp_dir};

#[derive(Clone)]
struct TestServer {
    root: PathBuf,
    channels: HashMap<ChannelId, Arc<tokio::sync::Mutex<Option<Channel<Msg>>>>>,
}

impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(
            channel.id(),
            Arc::new(tokio::sync::Mutex::new(Some(channel))),
        );
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channel = match self.channels.get(&id) {
            Some(slot) if name == "sftp" => slot.lock().await.take(),
            _ => None,
        };
        match channel {
            Some(channel) => {
                session.channel_success(id)?;
                let files = Files {
                    root: self.root.clone(),
                    open: HashMap::new(),
                    listings: HashMap::new(),
                    next: 0,
                };
                russh_sftp::server::run(channel.into_stream(), files).await;
            }
            None => session.channel_failure(id)?,
        }
        Ok(())
    }
}

/// SFTP handler backed by a real directory. Remote `/x` is `root/x`.
struct Files {
    root: PathBuf,
    open: HashMap<String, fs::File>,
    /// Directory handles and whether their entries were already sent.
    listings: HashMap<String, (PathBuf, bool)>,
    next: u32,
}

impl Files {
    fn local(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    fn handle(&mut self) -> String {
        self.next += 1;
        self.next.to_string()
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en-US".to_string(),
    }
}

fn code(err: std::io::Error) -> StatusCode {
    match err.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let path = if path == "." { "/".to_string() } else { path };
        Ok(Name {
            id,
            files: vec![File::dummy(path)],
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let local = self.local(&path);
        if !local.is_dir() {
            return Err(StatusCode::NoSuchFile);
        }
        let handle = self.handle();
        self.listings.insert(handle.clone(), (local, false));
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let (path, sent) = self.listings.get_mut(&handle).ok_or(StatusCode::Failure)?;
        if *sent {
            return Err(StatusCode::Eof);
        }
        *sent = true;
        let files = fs::read_dir(path)
            .map_err(code)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let attrs = FileAttributes::from(&entry.metadata().ok()?);
                Some(File::new(entry.file_name().to_string_lossy(), attrs))
            })
            .collect();
        Ok(Name { id, files })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let file = fs::OpenOptions::from(pflags)
            .open(self.local(&filename))
            .map_err(code)?;
        let handle = self.handle();
        self.open.insert(handle.clone(), file);
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let file = self.open.get_mut(&handle).ok_or(StatusCode::Failure)?;
        file.seek(SeekFrom::Start(offset)).map_err(code)?;
        let mut data = vec![0u8; len as usize];
        let read = file.read(&mut data).map_err(code)?;
        if read == 0 {
            return Err(StatusCode::Eof);
        }
        data.truncate(read);
        Ok(Data { id, data })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let file = self.open.get_mut(&handle).ok_or(StatusCode::Failure)?;
        file.seek(SeekFrom::Start(offset)).map_err(code)?;
        file.write_all(&data).map_err(code)?;
        Ok(ok(id))
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.open.remove(&handle);
        self.listings.remove(&handle);
        Ok(ok(id))
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let file = self.open.get(&handle).ok_or(StatusCode::Failure)?;
        let attrs = FileAttributes::from(&file.metadata().map_err(code)?);
        Ok(Attrs { id, attrs })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let attrs = FileAttributes::from(&fs::metadata(self.local(&path)).map_err(code)?);
        Ok(Attrs { id, attrs })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.stat(id, path).await
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        fs::create_dir(self.local(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        fs::remove_dir(self.local(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        fs::remove_file(self.local(&filename)).map_err(code)?;
        Ok(ok(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        fs::rename(self.local(&oldpath), self.local(&newpath)).map_err(code)?;
        Ok(ok(id))
    }
}

/// A connected SFTP session against a fresh server directory.
struct Fixture {
    handle: SftpHandle,
    /// Directory the server serves as `/`.
    remote: PathBuf,
    /// Scratch directory on the "local" side.
    local: PathBuf,
    next_id: u64,
}

impl Fixture {
    async fn connect(name: &str) -> Self {
        let remote = temp_dir(&format!("{name}-remote"));
        fs::write(remote.join("notes.txt"), b"hello").unwrap();
        fs::create_dir(remote.join("Docs")).unwrap();
        fs::write(remote.join("archive.bin"), vec![7u8; 300_000]).unwrap();

        let config = Arc::new(server::Config {
            auth_rejection_time: Duration::from_millis(10),
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
            ..Default::default()
        });
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut server = TestServer {
            root: remote.clone(),
            channels: HashMap::new(),
        };
        tokio::spawn(async move {
            let _ = server.run_on_socket(config, &listener).await;
        });

        let handle = spawn_sftp(
            &tokio::runtime::Handle::current(),
            vec![hop("a", port)],
            known_hosts(name),
            termker_core::secrets::SecretStore::disabled(),
            || {},
        );
        let mut fixture = Self {
            handle,
            remote,
            local: temp_dir(&format!("{name}-local")),
            next_id: 0,
        };
        let home = fixture
            .wait(|event| match event {
                Event::Connected { home } => Some(home.clone()),
                _ => None,
            })
            .await;
        assert_eq!(home, "/");
        fixture
    }

    /// Wait for an event `pick` accepts, answering prompts on the way.
    async fn wait<T>(&mut self, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
        let answers = Answers::new(true, &[PASSWORD]);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "timed out waiting for an event");
            match self.handle.events.try_recv() {
                Ok(Event::Prompt(prompt)) => answers.answer(prompt),
                Ok(Event::Closed { error }) => panic!("session closed: {error:?}"),
                Ok(event) => {
                    if let Some(value) = pick(&event) {
                        return value;
                    }
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
    }

    /// Names in the next listing of `path`.
    async fn listing(&mut self, path: &str) -> Vec<String> {
        self.wait(|event| match event {
            Event::Listing {
                path: listed,
                entries,
            } if listed == path => Some(entries.iter().map(|entry| entry.name.clone()).collect()),
            _ => None,
        })
        .await
    }

    async fn failure(&mut self) -> String {
        self.wait(|event| match event {
            Event::Failed(text) => Some(text.clone()),
            _ => None,
        })
        .await
    }

    fn transfer(&mut self, remote: &str, local: PathBuf) -> (u64, Transfer, Arc<AtomicBool>) {
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let transfer = Transfer {
            id: self.next_id,
            remote: remote.to_string(),
            local,
            cancel: cancel.clone(),
        };
        (self.next_id, transfer, cancel)
    }

    /// Wait for transfer `id` to end; returns its error and the last
    /// progress report seen for it.
    async fn done(&mut self, id: u64) -> (Option<String>, Option<(u64, u64)>) {
        let mut last = None;
        let error = self
            .wait(|event| match event {
                Event::Progress {
                    id: seen,
                    done,
                    total,
                } if *seen == id => {
                    last = Some((*done, *total));
                    None
                }
                Event::TransferDone { id: seen, error } if *seen == id => Some(error.clone()),
                _ => None,
            })
            .await;
        (error, last)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_home_with_folders_first() {
    let mut fixture = Fixture::connect("sftp-list").await;
    assert_eq!(
        fixture.listing("/").await,
        ["Docs", "archive.bin", "notes.txt"]
    );

    fixture.handle.send(Command::List("/missing".into()));
    assert!(
        fixture
            .failure()
            .await
            .starts_with("Could not open /missing")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn create_rename_and_delete_refresh_the_listing() {
    let mut fixture = Fixture::connect("sftp-ops").await;
    fixture.listing("/").await;

    fixture.handle.send(Command::CreateDir("/New".into()));
    assert!(fixture.listing("/").await.contains(&"New".to_string()));
    assert!(fixture.remote.join("New").is_dir());

    fixture.handle.send(Command::Rename {
        from: "/notes.txt".into(),
        to: "/Docs/renamed.txt".into(),
    });
    assert_eq!(fixture.listing("/Docs").await, ["renamed.txt"]);
    assert_eq!(
        fs::read(fixture.remote.join("Docs/renamed.txt")).unwrap(),
        b"hello"
    );

    // A folder that is not empty is not deleted.
    fixture.handle.send(Command::Remove {
        path: "/Docs".into(),
        is_dir: true,
    });
    assert!(fixture.failure().await.starts_with("Could not delete"));
    assert!(fixture.remote.join("Docs").is_dir());

    fixture.handle.send(Command::Remove {
        path: "/Docs/renamed.txt".into(),
        is_dir: false,
    });
    assert!(fixture.listing("/Docs").await.is_empty());
    fixture.handle.send(Command::Remove {
        path: "/New".into(),
        is_dir: true,
    });
    assert!(!fixture.listing("/").await.contains(&"New".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn download_writes_the_whole_file_and_reports_progress() {
    let mut fixture = Fixture::connect("sftp-download").await;
    let target = fixture.local.join("copy.bin");
    let (id, transfer, _) = fixture.transfer("/archive.bin", target.clone());
    fixture.handle.send(Command::Download(transfer));

    let (error, progress) = fixture.done(id).await;
    assert_eq!(error, None);
    assert_eq!(progress, Some((300_000, 300_000)));
    assert_eq!(fs::read(&target).unwrap(), vec![7u8; 300_000]);
    assert!(!partial_path(&target).exists(), "temporary file is gone");
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_download_leaves_no_file_behind() {
    let mut fixture = Fixture::connect("sftp-download-fail").await;
    let target = fixture.local.join("nothing.txt");
    let (id, transfer, _) = fixture.transfer("/does-not-exist", target.clone());
    fixture.handle.send(Command::Download(transfer));

    let (error, _) = fixture.done(id).await;
    assert!(error.is_some());
    assert!(!target.exists());
    assert!(!partial_path(&target).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_download_keeps_an_existing_local_file() {
    let mut fixture = Fixture::connect("sftp-cancel").await;
    let target = fixture.local.join("keep.bin");
    fs::write(&target, b"original").unwrap();
    let (id, transfer, cancel) = fixture.transfer("/archive.bin", target.clone());
    cancel.store(true, Ordering::Relaxed);
    fixture.handle.send(Command::Download(transfer));

    let (error, _) = fixture.done(id).await;
    assert_eq!(error.as_deref(), Some("Cancelled"));
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert!(!partial_path(&target).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn upload_creates_the_file_and_refreshes_the_folder() {
    let mut fixture = Fixture::connect("sftp-upload").await;
    fixture.listing("/").await;
    let source = fixture.local.join("report.pdf");
    let content: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    fs::write(&source, &content).unwrap();
    let (id, transfer, _) = fixture.transfer("/Docs/report.pdf", source);
    fixture.handle.send(Command::Upload(transfer));

    let (error, progress) = fixture.done(id).await;
    assert_eq!(error, None);
    assert_eq!(progress, Some((200_000, 200_000)));
    assert_eq!(fixture.listing("/Docs").await, ["report.pdf"]);
    assert_eq!(
        fs::read(fixture.remote.join("Docs/report.pdf")).unwrap(),
        content
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_upload_removes_the_partial_remote_file() {
    let mut fixture = Fixture::connect("sftp-upload-cancel").await;
    let source = fixture.local.join("big.bin");
    fs::write(&source, vec![1u8; 100_000]).unwrap();
    let (id, transfer, cancel) = fixture.transfer("/big.bin", source);
    cancel.store(true, Ordering::Relaxed);
    fixture.handle.send(Command::Upload(transfer));

    let (error, _) = fixture.done(id).await;
    assert_eq!(error.as_deref(), Some("Cancelled"));
    assert!(!fixture.listing("/").await.contains(&"big.bin".to_string()));
}

#[test]
fn remote_paths() {
    assert_eq!(join("/", "a"), "/a");
    assert_eq!(join("/home/me", "a"), "/home/me/a");
    assert_eq!(join("/home/me/", "a"), "/home/me/a");
    assert_eq!(parent("/"), None);
    assert_eq!(parent("/a"), Some("/".to_string()));
    assert_eq!(parent("/a/b/"), Some("/a".to_string()));
    assert_eq!(parent("relative"), None);
}

#[test]
fn names_that_would_escape_the_folder_are_rejected() {
    assert!(valid_name("report.pdf"));
    assert!(valid_name(".bashrc"));
    for bad in ["", ".", "..", "a/b", "../etc", "x\0y"] {
        assert!(!valid_name(bad), "{bad:?}");
    }
}

#[test]
fn sorting_and_formatting() {
    let entry = |name: &str, kind| Entry {
        name: name.to_string(),
        kind,
        size: 0,
        modified: None,
        permissions: None,
    };
    let mut entries = vec![
        entry("zeta.txt", EntryKind::File),
        entry("Alpha.txt", EntryKind::File),
        entry("src", EntryKind::Dir),
        entry("Bin", EntryKind::Dir),
    ];
    sort_entries(&mut entries);
    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["Bin", "src", "Alpha.txt", "zeta.txt"]);

    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(1023), "1023 B");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");

    assert_eq!(format_permissions(0o755), "rwxr-xr-x");
    assert_eq!(format_permissions(0o600), "rw-------");

    let at = |secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
    assert_eq!(format_time(at(0)), "1970-01-01 00:00");
    assert_eq!(format_time(at(951_782_400)), "2000-02-29 00:00");
    assert_eq!(format_time(at(1_794_925_664)), "2026-11-17 14:27");
}

#[test]
fn partial_file_sits_next_to_the_target() {
    let path = Path::new("/tmp/downloads/report.pdf");
    assert_eq!(
        partial_path(path),
        Path::new("/tmp/downloads/report.pdf.part")
    );
}
