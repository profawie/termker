//! Port forwarding over an SSH connection: local (-L), remote (-R) and
//! dynamic SOCKS5 (-D).

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use russh::client::Handle;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

use super::{
    ClientHandler, Connection, Hop, KnownHosts, Prompt, Prompter, SecretStore, SshError,
    connect_with, socks,
};

/// What to forward, with the addresses already validated.
#[derive(Debug, Clone, PartialEq)]
pub enum ForwardSpec {
    /// Listen locally; connections go out from the SSH host to `dest`.
    Local {
        bind: (String, u16),
        dest: (String, u16),
    },
    /// Listen on the SSH host; connections come back to `dest` from here.
    Remote {
        bind: (String, u16),
        dest: (String, u16),
    },
    /// Local SOCKS5 proxy; each connection goes out from the SSH host.
    Dynamic { bind: (String, u16) },
}

pub enum ForwardEvent {
    Status(String),
    Prompt(Prompt),
    /// The forward is up. `port` is the port actually bound, which matters
    /// when port 0 was requested.
    Active {
        port: u16,
    },
    /// Number of connections currently open through the forward.
    Connections(usize),
    Closed {
        error: Option<String>,
    },
}

/// Handle to a running forward. Dropping it stops the forward.
pub struct ForwardHandle {
    pub events: std::sync::mpsc::Receiver<ForwardEvent>,
    _stop: mpsc::Sender<()>,
}

type Emit = Arc<dyn Fn(ForwardEvent) + Send + Sync>;

/// Start a forward through `hops` in the background.
pub fn spawn_forward(
    runtime: &tokio::runtime::Handle,
    hops: Vec<Hop>,
    known_hosts: KnownHosts,
    secrets: SecretStore,
    spec: ForwardSpec,
    wake: impl Fn() + Send + Sync + 'static,
) -> ForwardHandle {
    let (event_tx, events) = std::sync::mpsc::channel();
    let (stop, mut stopped) = mpsc::channel::<()>(1);
    let emit: Emit = Arc::new(move |event| {
        let _ = event_tx.send(event);
        wake();
    });

    runtime.spawn(async move {
        let result = tokio::select! {
            result = run(&hops, &known_hosts, secrets, &spec, &emit) => result,
            // `recv` returns `None` once the handle is dropped.
            _ = stopped.recv() => Ok(()),
        };
        emit(ForwardEvent::Closed {
            error: match result {
                Ok(()) | Err(SshError::Cancelled) => None,
                Err(err) => Some(err.to_string()),
            },
        });
    });

    ForwardHandle {
        events,
        _stop: stop,
    }
}

async fn run(
    hops: &[Hop],
    known_hosts: &KnownHosts,
    secrets: SecretStore,
    spec: &ForwardSpec,
    emit: &Emit,
) -> Result<(), SshError> {
    let prompter = {
        let emit = emit.clone();
        Prompter::new(move |prompt| emit(ForwardEvent::Prompt(prompt)), secrets)
    };
    let status = {
        let emit = emit.clone();
        move |text| emit(ForwardEvent::Status(text))
    };
    let (forwarded_tx, mut forwarded_rx) = mpsc::unbounded_channel();
    let forwarded = matches!(spec, ForwardSpec::Remote { .. }).then_some(forwarded_tx);
    let connection =
        Arc::new(connect_with(hops, known_hosts, &prompter, &status, forwarded).await?);
    let target = connection.target();
    let counter = Counter {
        open: Arc::new(AtomicUsize::new(0)),
        emit: emit.clone(),
    };

    match spec {
        ForwardSpec::Local { bind, dest } => {
            let listener = listen(bind).await?;
            emit(ForwardEvent::Active {
                port: listener.local_addr()?.port(),
            });
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, peer) = accepted?;
                        let channel = target
                            .channel_open_direct_tcpip(
                                dest.0.clone(),
                                dest.1 as u32,
                                peer.ip().to_string(),
                                peer.port() as u32,
                            )
                            .await;
                        // A refused destination closes this one connection,
                        // not the whole forward.
                        if let Ok(channel) = channel {
                            pipe(stream, channel.into_stream(), counter.clone());
                        }
                    }
                    _ = closed(target) => return Err(SshError::Forward("The SSH connection was lost.".into())),
                }
            }
        }
        ForwardSpec::Dynamic { bind } => {
            let listener = listen(bind).await?;
            emit(ForwardEvent::Active {
                port: listener.local_addr()?.port(),
            });
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (stream, peer) = accepted?;
                        tokio::spawn(socks_connection(stream, peer, connection.clone(), counter.clone()));
                    }
                    _ = closed(target) => return Err(SshError::Forward("The SSH connection was lost.".into())),
                }
            }
        }
        ForwardSpec::Remote { bind, dest } => {
            let port = target
                .tcpip_forward(bind.0.clone(), bind.1 as u32)
                .await
                .map_err(|_| {
                    SshError::Forward(format!(
                        "The server refused to listen on {}:{}.",
                        bind.0, bind.1
                    ))
                })?;
            emit(ForwardEvent::Active {
                port: if port == 0 { bind.1 } else { port as u16 },
            });
            loop {
                tokio::select! {
                    channel = forwarded_rx.recv() => {
                        let Some(channel) = channel else {
                            return Err(SshError::Forward("The SSH connection was lost.".into()));
                        };
                        let dest = dest.clone();
                        let counter = counter.clone();
                        tokio::spawn(async move {
                            if let Ok(stream) = TcpStream::connect((dest.0.as_str(), dest.1)).await {
                                pipe(stream, channel.into_stream(), counter);
                            }
                        });
                    }
                    _ = closed(target) => return Err(SshError::Forward("The SSH connection was lost.".into())),
                }
            }
        }
    }
}

async fn listen(bind: &(String, u16)) -> Result<TcpListener, SshError> {
    TcpListener::bind((bind.0.as_str(), bind.1))
        .await
        .map_err(|err| {
            SshError::Forward(format!("Could not listen on {}:{}: {err}", bind.0, bind.1))
        })
}

/// Resolves when the SSH connection is gone.
async fn closed(handle: &Handle<ClientHandler>) {
    while !handle.is_closed() {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

/// Serve one SOCKS5 client: read where it wants to go, open a channel to
/// that destination from the SSH host, then relay.
async fn socks_connection(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    connection: Arc<Connection>,
    counter: Counter,
) {
    let Ok((host, port)) = socks::accept(&mut stream).await else {
        return;
    };
    let opened = connection
        .target()
        .channel_open_direct_tcpip(host, port as u32, peer.ip().to_string(), peer.port() as u32)
        .await;
    match opened {
        Ok(channel) => {
            if socks::reply(&mut stream, socks::REPLY_SUCCEEDED)
                .await
                .is_ok()
            {
                pipe(stream, channel.into_stream(), counter);
            }
        }
        Err(russh::Error::ChannelOpenFailure(_)) => {
            let _ = socks::reply(&mut stream, socks::REPLY_HOST_UNREACHABLE).await;
        }
        Err(_) => {
            let _ = socks::reply(&mut stream, socks::REPLY_GENERAL_FAILURE).await;
        }
    }
}

/// Tracks open connections and reports the count to the UI.
#[derive(Clone)]
struct Counter {
    open: Arc<AtomicUsize>,
    emit: Emit,
}

/// Copy bytes both ways until either side closes.
fn pipe<A, B>(mut a: A, mut b: B, counter: Counter)
where
    A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    B: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let open = counter.open.fetch_add(1, Ordering::SeqCst) + 1;
        (counter.emit)(ForwardEvent::Connections(open));
        let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
        let open = counter.open.fetch_sub(1, Ordering::SeqCst) - 1;
        (counter.emit)(ForwardEvent::Connections(open));
    });
}
