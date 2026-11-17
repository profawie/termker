//! End-to-end tests against an in-process SSH server (russh::server).

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server::{self, Auth, Msg, Server as _, Session},
};
use tokio::net::TcpListener;

use super::*;

const PASSWORD: &str = "correct";

#[derive(Clone, Default)]
struct TestServer {
    /// Number of direct-tcpip channels opened through this server.
    forwards: Arc<AtomicUsize>,
    authorized_key: Option<russh::keys::PublicKey>,
    /// Channels of this client that carry forwarded TCP, not a shell.
    forwarded: Vec<ChannelId>,
    /// Result of trying to open a forwarded-tcpip channel nobody asked for:
    /// `Some(true)` if the client accepted it.
    unsolicited_forward_accepted: Arc<Mutex<Option<bool>>>,
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

    async fn auth_publickey(
        &mut self,
        _user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        let accepted = self
            .authorized_key
            .as_ref()
            .is_some_and(|known| known.key_data() == key.public_key.key_data());
        Ok(if accepted {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _cols: u32,
        _rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        session.data(channel, b"ready\r\n".to_vec())?;
        // Act like a hostile server: try to open a connection back to the
        // client although no remote forward was requested.
        let handle = session.handle();
        let outcome = self.unsolicited_forward_accepted.clone();
        tokio::spawn(async move {
            let accepted = handle
                .channel_open_forwarded_tcpip("127.0.0.1", 1, "127.0.0.1", 2)
                .await
                .is_ok();
            *outcome.lock().unwrap() = Some(accepted);
        });
        Ok(())
    }

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let Ok(listener) = TcpListener::bind((address, *port as u16)).await else {
            return Ok(false);
        };
        let bound = listener.local_addr().unwrap().port() as u32;
        *port = bound;
        let address = address.to_string();
        let handle = session.handle();
        tokio::spawn(async move {
            while let Ok((mut stream, peer)) = listener.accept().await {
                let opened = handle
                    .channel_open_forwarded_tcpip(
                        address.clone(),
                        bound,
                        peer.ip().to_string(),
                        peer.port() as u32,
                    )
                    .await;
                let Ok(channel) = opened else { break };
                tokio::spawn(async move {
                    let mut channel = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut channel).await;
                });
            }
        });
        Ok(true)
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        cols: u32,
        rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, format!("size {cols}x{rows}\r\n").into_bytes())?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.forwarded.contains(&channel) {
            return Ok(());
        }
        let mut reply = b"echo:".to_vec();
        reply.extend_from_slice(data);
        session.data(channel, reply)?;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _originator: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.forwards.fetch_add(1, Ordering::SeqCst);
        let Ok(mut upstream) = tokio::net::TcpStream::connect((host, port as u16)).await else {
            return Ok(());
        };
        self.forwarded.push(channel.id());
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
        });
        Ok(())
    }
}

/// Start the server on a random port and return that port.
async fn start(server: TestServer) -> u16 {
    let config = Arc::new(server::Config {
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        ..Default::default()
    });
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut server = server;
        let _ = server.run_on_socket(config, &listener).await;
    });
    port
}

fn hop(name: &str, port: u16) -> Hop {
    Hop {
        name: name.into(),
        address: "127.0.0.1".into(),
        port,
        username: "tester".into(),
        auth: AuthMethod::Password,
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rterminalfy-ssh-{}-{name}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn known_hosts(name: &str) -> KnownHosts {
    KnownHosts::new(temp_dir(name).join("known_hosts"))
}

/// Automatic answers to prompts, recording what was asked.
#[derive(Clone)]
struct Answers {
    accept_host_key: bool,
    passwords: Arc<Mutex<Vec<&'static str>>>,
    passphrase: Option<&'static str>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Answers {
    fn new(accept_host_key: bool, passwords: &[&'static str]) -> Self {
        Self {
            accept_host_key,
            passwords: Arc::new(Mutex::new(passwords.iter().rev().copied().collect())),
            passphrase: None,
            asked: Arc::default(),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    fn prompter(&self) -> Prompter {
        let this = self.clone();
        Prompter::new(move |prompt| this.answer(prompt))
    }

    fn answer(&self, prompt: Prompt) {
        let mut asked = self.asked.lock().unwrap();
        match prompt {
            Prompt::HostKey {
                hop, status, reply, ..
            } => {
                asked.push(format!("hostkey:{hop}:{status:?}"));
                let _ = reply.send(self.accept_host_key);
            }
            Prompt::Password {
                hop, retry, reply, ..
            } => {
                asked.push(format!("password:{hop}:retry={retry}"));
                let next = self.passwords.lock().unwrap().pop();
                let _ = reply.send(next.map(str::to_string));
            }
            Prompt::Passphrase { retry, reply, .. } => {
                asked.push(format!("passphrase:retry={retry}"));
                let _ = reply.send(self.passphrase.map(str::to_string));
            }
        }
    }
}

fn quiet(_: String) {}

#[tokio::test]
async fn first_connection_asks_about_host_key_and_remembers_it() {
    let port = start(TestServer::default()).await;
    let known = known_hosts("tofu");
    let hops = [hop("a", port)];

    let answers = Answers::new(true, &[PASSWORD]);
    let connection = connect(&hops, &known, &answers.prompter(), &quiet)
        .await
        .unwrap();
    connection.close().await;
    assert_eq!(
        answers.asked(),
        ["hostkey:a:Unknown", "password:a:retry=false"]
    );

    // Second time: the key is trusted, so no host key prompt.
    let answers = Answers::new(false, &[PASSWORD]);
    let connection = connect(&hops, &known, &answers.prompter(), &quiet)
        .await
        .unwrap();
    connection.close().await;
    assert_eq!(answers.asked(), ["password:a:retry=false"]);
}

#[tokio::test]
async fn rejected_host_key_stops_before_any_password_is_asked() {
    let port = start(TestServer::default()).await;
    let known = known_hosts("reject");
    let answers = Answers::new(false, &[PASSWORD]);
    let result = connect(&[hop("a", port)], &known, &answers.prompter(), &quiet).await;
    assert!(matches!(result, Err(SshError::HostKeyRejected { .. })));
    assert_eq!(answers.asked(), ["hostkey:a:Unknown"]);
    // A rejected key is not saved.
    let answers = Answers::new(false, &[PASSWORD]);
    let _ = connect(&[hop("a", port)], &known, &answers.prompter(), &quiet).await;
    assert_eq!(answers.asked(), ["hostkey:a:Unknown"]);
}

#[tokio::test]
async fn changed_host_key_is_reported_as_changed() {
    let known = known_hosts("changed");
    let other = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let port = start(TestServer::default()).await;
    known.trust("127.0.0.1", port, other.public_key()).unwrap();

    let answers = Answers::new(false, &[PASSWORD]);
    let result = connect(&[hop("a", port)], &known, &answers.prompter(), &quiet).await;
    assert!(matches!(result, Err(SshError::HostKeyRejected { .. })));
    assert_eq!(answers.asked(), ["hostkey:a:Changed"]);
}

#[tokio::test]
async fn wrong_password_is_retried_then_fails() {
    let port = start(TestServer::default()).await;
    let known = known_hosts("retry");

    let answers = Answers::new(true, &["wrong", PASSWORD]);
    let connection = connect(&[hop("a", port)], &known, &answers.prompter(), &quiet)
        .await
        .unwrap();
    connection.close().await;
    assert_eq!(
        answers.asked()[1..],
        ["password:a:retry=false", "password:a:retry=true"]
    );

    let answers = Answers::new(true, &["wrong", "wrong", "wrong"]);
    let result = connect(&[hop("a", port)], &known, &answers.prompter(), &quiet).await;
    assert!(matches!(result, Err(SshError::AuthFailed { .. })));
    assert_eq!(answers.asked().len(), 3);
}

#[tokio::test]
async fn cancelling_the_password_prompt_cancels_the_connection() {
    let port = start(TestServer::default()).await;
    let answers = Answers::new(true, &[]);
    let result = connect(
        &[hop("a", port)],
        &known_hosts("cancel"),
        &answers.prompter(),
        &quiet,
    )
    .await;
    assert!(matches!(result, Err(SshError::Cancelled)));
}

#[tokio::test]
async fn missing_username_is_reported_without_connecting() {
    let mut hop = hop("a", 1);
    hop.username.clear();
    let answers = Answers::new(true, &[]);
    let result = connect(&[hop], &known_hosts("nouser"), &answers.prompter(), &quiet).await;
    assert!(matches!(result, Err(SshError::MissingUsername { .. })));
    assert!(answers.asked().is_empty());
}

#[tokio::test]
async fn target_is_reached_through_the_jump_host() {
    let jump = TestServer::default();
    let target = TestServer::default();
    let jump_port = start(jump.clone()).await;
    let target_port = start(target.clone()).await;
    let known = known_hosts("jump");

    let answers = Answers::new(true, &[PASSWORD, PASSWORD]);
    let hops = [hop("jump", jump_port), hop("target", target_port)];
    let connection = connect(&hops, &known, &answers.prompter(), &quiet)
        .await
        .unwrap();
    assert_eq!(
        answers.asked(),
        [
            "hostkey:jump:Unknown",
            "password:jump:retry=false",
            "hostkey:target:Unknown",
            "password:target:retry=false",
        ]
    );
    assert_eq!(jump.forwards.load(Ordering::SeqCst), 1);
    assert_eq!(target.forwards.load(Ordering::SeqCst), 0);

    // A session on the target really works through the tunnel.
    let mut channel = connection.target().channel_open_session().await.unwrap();
    channel.request_shell(true).await.unwrap();
    channel.data_bytes(b"hello".to_vec()).await.unwrap();
    let mut seen = Vec::new();
    while !seen.ends_with(b"echo:hello") {
        match tokio::time::timeout(Duration::from_secs(5), channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => seen.extend_from_slice(&data),
            Ok(Some(_)) => {}
            other => panic!("channel ended early: {:?}", other.is_ok()),
        }
    }
    connection.close().await;
}

#[tokio::test]
async fn key_file_auth_with_and_without_passphrase() {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let port = start(TestServer {
        authorized_key: Some(key.public_key().clone()),
        ..Default::default()
    })
    .await;
    let dir = temp_dir("keys");
    let known = KnownHosts::new(dir.join("known_hosts"));

    let plain = dir.join("id_plain");
    std::fs::write(
        &plain,
        key.to_openssh(Default::default()).unwrap().as_bytes(),
    )
    .unwrap();
    let encrypted = dir.join("id_encrypted");
    let locked = key.encrypt(&mut rand::rng(), "secret").unwrap();
    std::fs::write(
        &encrypted,
        locked.to_openssh(Default::default()).unwrap().as_bytes(),
    )
    .unwrap();

    let with_key = |path: &std::path::Path| Hop {
        auth: AuthMethod::KeyFile {
            path: path.display().to_string(),
        },
        ..hop("a", port)
    };

    let answers = Answers::new(true, &[]);
    let connection = connect(&[with_key(&plain)], &known, &answers.prompter(), &quiet)
        .await
        .unwrap();
    connection.close().await;
    assert_eq!(answers.asked(), ["hostkey:a:Unknown"]);

    let mut answers = Answers::new(true, &[]);
    answers.passphrase = Some("secret");
    let connection = connect(&[with_key(&encrypted)], &known, &answers.prompter(), &quiet)
        .await
        .unwrap();
    connection.close().await;
    assert_eq!(answers.asked(), ["passphrase:retry=false"]);

    let mut answers = Answers::new(true, &[]);
    answers.passphrase = Some("wrong");
    let result = connect(&[with_key(&encrypted)], &known, &answers.prompter(), &quiet).await;
    assert!(matches!(result, Err(SshError::Key { .. })));
    assert_eq!(answers.asked().len(), 3);

    // A key the server does not authorize is rejected.
    let stranger = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let path = dir.join("id_stranger");
    std::fs::write(
        &path,
        stranger.to_openssh(Default::default()).unwrap().as_bytes(),
    )
    .unwrap();
    let result = connect(
        &[with_key(&path)],
        &known,
        &Answers::new(true, &[]).prompter(),
        &quiet,
    )
    .await;
    assert!(matches!(result, Err(SshError::AuthFailed { .. })));
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_session_streams_output_input_and_resize() {
    let port = start(TestServer::default()).await;
    let answers = Answers::new(true, &[PASSWORD]);
    let session = spawn_shell(
        &tokio::runtime::Handle::current(),
        vec![hop("a", port)],
        known_hosts("shell"),
        (80, 24),
        || {},
    );

    let mut output = Vec::new();
    let mut connected = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut sent = false;
    while !(output.ends_with(b"echo:ls")
        && String::from_utf8_lossy(&output).contains("size 100x30"))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out: {output:?}"
        );
        match session.events.recv_timeout(Duration::from_millis(50)) {
            Ok(SessionEvent::Prompt(prompt)) => answers.answer(prompt),
            Ok(SessionEvent::Connected) => connected = true,
            Ok(SessionEvent::Output(bytes)) => output.extend_from_slice(&bytes),
            Ok(SessionEvent::Closed { error }) => panic!("closed: {error:?}"),
            Ok(SessionEvent::Status(_)) | Err(_) => {}
        }
        if connected && !sent && output.ends_with(b"ready\r\n") {
            session.send(SessionCommand::Resize {
                cols: 100,
                rows: 30,
            });
            session.send(SessionCommand::Input(b"ls".to_vec()));
            sent = true;
        }
        tokio::task::yield_now().await;
    }
    assert!(output.starts_with(b"ready\r\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_session_reports_connection_errors() {
    // A port with no server behind it.
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let session = spawn_shell(
        &tokio::runtime::Handle::current(),
        vec![hop("a", port)],
        known_hosts("refused"),
        (80, 24),
        || {},
    );
    loop {
        match session
            .events
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
        {
            SessionEvent::Closed { error } => {
                assert!(error.unwrap().starts_with("Network error"));
                break;
            }
            SessionEvent::Status(_) => {}
            _ => panic!("unexpected event"),
        }
    }
}

mod forwarding {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    use super::*;
    use crate::ssh::forward::{ForwardEvent, ForwardHandle, ForwardSpec, spawn_forward};

    /// TCP server that sends back whatever it receives. Returns its port.
    async fn echo_server() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut reader, mut writer) = stream.split();
                    let _ = tokio::io::copy(&mut reader, &mut writer).await;
                });
            }
        });
        port
    }

    fn local(port: u16) -> (String, u16) {
        ("127.0.0.1".to_string(), port)
    }

    fn start_forward(ssh_port: u16, name: &str, spec: ForwardSpec) -> ForwardHandle {
        spawn_forward(
            &tokio::runtime::Handle::current(),
            vec![hop("a", ssh_port)],
            known_hosts(name),
            spec,
            || {},
        )
    }

    /// Answer prompts until the forward is up (returns its port) or closes
    /// (returns the error text).
    async fn until_active(handle: &ForwardHandle) -> Result<u16, String> {
        let answers = Answers::new(true, &[PASSWORD]);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline, "timed out");
            match handle.events.try_recv() {
                Ok(ForwardEvent::Prompt(prompt)) => answers.answer(prompt),
                Ok(ForwardEvent::Active { port }) => return Ok(port),
                Ok(ForwardEvent::Closed { error }) => return Err(error.unwrap_or_default()),
                Ok(_) => {}
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    }

    async fn assert_echo(stream: &mut TcpStream, text: &[u8]) {
        stream.write_all(text).await.unwrap();
        let mut reply = vec![0u8; text.len()];
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut reply))
            .await
            .expect("echo arrives in time")
            .unwrap();
        assert_eq!(reply, text);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_forward_reaches_the_destination_through_ssh() {
        let server = TestServer::default();
        let ssh_port = start(server.clone()).await;
        let echo = echo_server().await;
        let handle = start_forward(
            ssh_port,
            "fwd-local",
            ForwardSpec::Local {
                bind: local(0),
                dest: local(echo),
            },
        );
        let port = until_active(&handle).await.unwrap();

        let mut first = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut second = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        assert_echo(&mut first, b"one").await;
        assert_echo(&mut second, b"two").await;
        assert_eq!(server.forwards.load(Ordering::SeqCst), 2);

        // Dropping the handle stops listening.
        drop(handle);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            assert!(std::time::Instant::now() < deadline, "listener still open");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dynamic_forward_speaks_socks5() {
        let ssh_port = start(TestServer::default()).await;
        let echo = echo_server().await;
        let handle = start_forward(
            ssh_port,
            "fwd-dynamic",
            ForwardSpec::Dynamic { bind: local(0) },
        );
        let port = until_active(&handle).await.unwrap();

        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut request = vec![5, 1, 0, 5, 1, 0, 3, 9];
        request.extend_from_slice(b"localhost");
        request.extend_from_slice(&echo.to_be_bytes());
        stream.write_all(&request).await.unwrap();
        let mut reply = [0u8; 12];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[..4], [5, 0, 5, 0], "method chosen, then success");
        assert_echo(&mut stream, b"through socks").await;

        // A destination nothing listens on is reported as unreachable.
        let closed = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let dead_port = closed.local_addr().unwrap().port();
        drop(closed);
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut request = vec![5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1];
        request.extend_from_slice(&dead_port.to_be_bytes());
        stream.write_all(&request).await.unwrap();
        let mut reply = [0u8; 12];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply[2..4], [5, 4]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remote_forward_brings_connections_back() {
        let ssh_port = start(TestServer::default()).await;
        let echo = echo_server().await;
        let handle = start_forward(
            ssh_port,
            "fwd-remote",
            ForwardSpec::Remote {
                bind: local(0),
                dest: local(echo),
            },
        );
        let port = until_active(&handle).await.unwrap();
        assert_ne!(port, 0, "the port chosen by the server is reported");

        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        assert_echo(&mut stream, b"from the far side").await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn busy_local_port_is_reported() {
        let ssh_port = start(TestServer::default()).await;
        let taken = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let handle = start_forward(
            ssh_port,
            "fwd-busy",
            ForwardSpec::Local {
                bind: local(taken.local_addr().unwrap().port()),
                dest: local(1),
            },
        );
        let error = until_active(&handle).await.unwrap_err();
        assert!(
            error.starts_with("Could not listen on 127.0.0.1:"),
            "{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shell_sessions_refuse_unrequested_connections_from_the_server() {
        let server = TestServer::default();
        let port = start(server.clone()).await;
        let answers = Answers::new(true, &[PASSWORD]);
        let session = spawn_shell(
            &tokio::runtime::Handle::current(),
            vec![hop("a", port)],
            known_hosts("unsolicited"),
            (80, 24),
            || {},
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline, "timed out");
            if let Ok(SessionEvent::Prompt(prompt)) = session.events.try_recv() {
                answers.answer(prompt);
            }
            if let Some(accepted) = *server.unsolicited_forward_accepted.lock().unwrap() {
                assert!(!accepted, "the client must reject the channel");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
