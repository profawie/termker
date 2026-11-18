//! One open terminal tab: a background SSH session plus the emulator that
//! holds its screen.

use uuid::Uuid;

use termker_core::secrets::SecretStore;
use termker_ssh::{
    self as ssh, Hop, KnownHosts, Prompt, SessionCommand, SessionEvent, SessionHandle,
};

use crate::emulator::{Emulator, Size};

const INITIAL_SIZE: Size = Size { cols: 80, rows: 24 };

pub enum SessionState {
    /// Still connecting; the text says what is happening right now.
    Connecting(String),
    Connected,
    /// Ended, with the reason if it did not close normally.
    Closed(Option<String>),
}

pub struct Session {
    pub id: Uuid,
    pub host_id: Uuid,
    pub name: String,
    pub hops: Vec<Hop>,
    pub state: SessionState,
    pub emulator: Emulator,
    /// Question waiting for the user, if any.
    pub prompt: Option<Prompt>,
    /// Text typed into the current password or passphrase prompt.
    pub secret_input: String,
    /// Fraction of a line left over from smooth scrolling.
    pub scroll_remainder: f32,
    handle: SessionHandle,
}

impl Session {
    /// Start connecting through `hops` (target last).
    pub fn open(
        runtime: &tokio::runtime::Handle,
        host_id: Uuid,
        hops: Vec<Hop>,
        known_hosts: KnownHosts,
        secrets: SecretStore,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let name = hops.last().map(|hop| hop.name.clone()).unwrap_or_default();
        let handle = ssh::spawn_shell(
            runtime,
            hops.clone(),
            known_hosts,
            secrets,
            (INITIAL_SIZE.cols, INITIAL_SIZE.rows),
            wake,
        );
        Self {
            id: Uuid::new_v4(),
            host_id,
            name,
            hops,
            state: SessionState::Connecting("Starting".to_string()),
            emulator: Emulator::new(INITIAL_SIZE),
            prompt: None,
            secret_input: String::new(),
            scroll_remainder: 0.0,
            handle,
        }
    }

    /// Route as text, e.g. `bastion > web-01`.
    pub fn route(&self) -> String {
        self.hops
            .iter()
            .map(|hop| hop.name.as_str())
            .collect::<Vec<_>>()
            .join(" > ")
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.state, SessionState::Connected)
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.state, SessionState::Closed(_))
    }

    /// Apply everything the background session reported since the last call.
    pub fn poll(&mut self) {
        while let Ok(event) = self.handle.events.try_recv() {
            match event {
                SessionEvent::Status(text) => {
                    if !self.is_connected() {
                        self.state = SessionState::Connecting(text);
                    }
                }
                SessionEvent::Prompt(prompt) => {
                    self.secret_input.clear();
                    self.prompt = Some(prompt);
                }
                SessionEvent::Connected => {
                    self.state = SessionState::Connected;
                    self.prompt = None;
                    // The size may have changed while connecting.
                    let size = self.emulator.size();
                    self.handle.send(SessionCommand::Resize {
                        cols: size.cols,
                        rows: size.rows,
                    });
                }
                SessionEvent::Output(bytes) => {
                    let replies = self.emulator.feed(&bytes);
                    if !replies.is_empty() {
                        self.handle.send(SessionCommand::Input(replies));
                    }
                }
                SessionEvent::Closed { error } => {
                    self.prompt = None;
                    self.secret_input.clear();
                    self.state = SessionState::Closed(error);
                }
            }
        }
    }

    pub fn send_input(&mut self, bytes: Vec<u8>) {
        if self.is_connected() && !bytes.is_empty() {
            self.handle.send(SessionCommand::Input(bytes));
        }
    }

    pub fn resize(&mut self, size: Size) {
        if self.emulator.resize(size) && self.is_connected() {
            self.handle.send(SessionCommand::Resize {
                cols: size.cols,
                rows: size.rows,
            });
        }
    }
}
