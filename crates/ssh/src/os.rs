//! Finding out which operating system the remote host runs.
//!
//! One short command is run on its own channel, next to the user's shell and
//! never inside it, so nothing shows up in the terminal or in the shell
//! history. Hosts that refuse it (network devices, restricted accounts,
//! `MaxSessions 1`) simply stay unidentified.

use std::time::Duration;

use russh::{ChannelMsg, client::Handle};

use termker_core::os::{self, RemoteOs};

use crate::ClientHandler;

/// Asked through `sh` so it also works when the login shell is csh or fish.
/// Every part may be missing: `uname` exists on every Unix, `os-release` on
/// Linux and newer BSDs, `sw_vers` only on macOS.
const PROBE: &str = "sh -c 'echo \"TERMKER_UNAME=$(uname -sr 2>/dev/null)\"; \
     cat /etc/os-release 2>/dev/null || cat /usr/lib/os-release 2>/dev/null; \
     sw_vers 2>/dev/null'";

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// More than any honest answer needs.
const MAX_OUTPUT: usize = 16 * 1024;

/// Ask the host what it runs. `None` if it will not say, takes too long or
/// answers something that is not understood.
pub async fn detect(handle: &Handle<ClientHandler>) -> Option<RemoteOs> {
    let output = tokio::time::timeout(PROBE_TIMEOUT, run_probe(handle))
        .await
        .ok()??;
    os::parse(&String::from_utf8_lossy(&output))
}

async fn run_probe(handle: &Handle<ClientHandler>) -> Option<Vec<u8>> {
    let mut channel = handle.channel_open_session().await.ok()?;
    channel.exec(true, PROBE).await.ok()?;
    let mut output = Vec::new();
    while let Some(message) = channel.wait().await {
        match message {
            ChannelMsg::Data { data } => {
                output.extend_from_slice(&data);
                if output.len() > MAX_OUTPUT {
                    break;
                }
            }
            ChannelMsg::Failure | ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    let _ = channel.close().await;
    Some(output)
}
