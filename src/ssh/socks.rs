//! Minimal SOCKS5 server side (RFC 1928) for dynamic port forwarding:
//! no authentication, CONNECT only.

use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const VERSION: u8 = 5;
const NO_AUTH: u8 = 0;
const NO_ACCEPTABLE_METHOD: u8 = 0xff;
const CMD_CONNECT: u8 = 1;

pub const REPLY_SUCCEEDED: u8 = 0;
pub const REPLY_GENERAL_FAILURE: u8 = 1;
pub const REPLY_HOST_UNREACHABLE: u8 = 4;
const REPLY_COMMAND_NOT_SUPPORTED: u8 = 7;
const REPLY_ADDRESS_NOT_SUPPORTED: u8 = 8;

#[derive(Debug, PartialEq)]
pub enum SocksError {
    Io(std::io::ErrorKind),
    /// Not a SOCKS5 client, or a request this server does not support.
    Unsupported(&'static str),
}

impl From<std::io::Error> for SocksError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.kind())
    }
}

/// Run the handshake and read the CONNECT request. Returns the destination
/// host and port. The caller must then send [`reply`].
pub async fn accept<S>(stream: &mut S) -> Result<(String, u16), SocksError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    if head[0] != VERSION {
        return Err(SocksError::Unsupported("not a SOCKS5 client"));
    }
    let mut methods = vec![0u8; head[1] as usize];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&NO_AUTH) {
        stream.write_all(&[VERSION, NO_ACCEPTABLE_METHOD]).await?;
        return Err(SocksError::Unsupported("client requires authentication"));
    }
    stream.write_all(&[VERSION, NO_AUTH]).await?;

    let mut request = [0u8; 4];
    stream.read_exact(&mut request).await?;
    let [version, command, _reserved, address_type] = request;
    if version != VERSION {
        return Err(SocksError::Unsupported("not a SOCKS5 request"));
    }
    if command != CMD_CONNECT {
        reply(stream, REPLY_COMMAND_NOT_SUPPORTED).await?;
        return Err(SocksError::Unsupported("only CONNECT is supported"));
    }
    let host = match address_type {
        1 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).await?;
            Ipv4Addr::from(octets).to_string()
        }
        3 => {
            let length = stream.read_u8().await? as usize;
            let mut name = vec![0u8; length];
            stream.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| SocksError::Unsupported("invalid host name"))?
        }
        4 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).await?;
            Ipv6Addr::from(octets).to_string()
        }
        _ => {
            reply(stream, REPLY_ADDRESS_NOT_SUPPORTED).await?;
            return Err(SocksError::Unsupported("unknown address type"));
        }
    };
    let port = stream.read_u16().await?;
    Ok((host, port))
}

/// Send the reply to the CONNECT request. The bound address is reported as
/// 0.0.0.0:0, which clients ignore for CONNECT.
pub async fn reply<S>(stream: &mut S, code: u8) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&[VERSION, code, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `input` to `accept` and return its result plus what it wrote.
    async fn run(input: &[u8]) -> (Result<(String, u16), SocksError>, Vec<u8>) {
        let (mut client, mut server) = tokio::io::duplex(1024);
        client.write_all(input).await.unwrap();
        // Close our sending side so a short request ends instead of hanging.
        client.shutdown().await.unwrap();
        let result = accept(&mut server).await;
        drop(server);
        let mut written = Vec::new();
        client.read_to_end(&mut written).await.unwrap();
        (result, written)
    }

    #[tokio::test]
    async fn connect_to_domain_name() {
        let mut input = vec![5, 1, 0, 5, 1, 0, 3, 11];
        input.extend_from_slice(b"example.com");
        input.extend_from_slice(&443u16.to_be_bytes());
        let (result, written) = run(&input).await;
        assert_eq!(result, Ok(("example.com".to_string(), 443)));
        assert_eq!(written, [5, 0]);
    }

    #[tokio::test]
    async fn connect_to_ipv4_and_ipv6() {
        let (result, _) = run(&[5, 1, 0, 5, 1, 0, 1, 10, 0, 0, 7, 0, 22]).await;
        assert_eq!(result, Ok(("10.0.0.7".to_string(), 22)));

        let mut input = vec![5, 2, 2, 0, 5, 1, 0, 4];
        input.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        input.extend_from_slice(&8080u16.to_be_bytes());
        let (result, _) = run(&input).await;
        assert_eq!(result, Ok(("::1".to_string(), 8080)));
    }

    #[tokio::test]
    async fn rejects_other_protocols_and_auth_only_clients() {
        let (result, written) = run(b"GET / HTTP/1.1\r\n").await;
        assert!(matches!(result, Err(SocksError::Unsupported(_))));
        assert!(written.is_empty());

        // Client offers only username/password (method 2).
        let (result, written) = run(&[5, 1, 2]).await;
        assert!(matches!(result, Err(SocksError::Unsupported(_))));
        assert_eq!(written, [5, 0xff]);
    }

    #[tokio::test]
    async fn rejects_bind_and_unknown_address_types() {
        let (result, written) = run(&[5, 1, 0, 5, 2, 0, 1, 1, 2, 3, 4, 0, 80]).await;
        assert!(matches!(result, Err(SocksError::Unsupported(_))));
        assert_eq!(written[2..4], [5, REPLY_COMMAND_NOT_SUPPORTED]);

        let (result, written) = run(&[5, 1, 0, 5, 1, 0, 9]).await;
        assert!(matches!(result, Err(SocksError::Unsupported(_))));
        assert_eq!(written[2..4], [5, REPLY_ADDRESS_NOT_SUPPORTED]);
    }

    #[tokio::test]
    async fn truncated_request_is_an_io_error() {
        let (result, _) = run(&[5, 1, 0, 5, 1]).await;
        assert_eq!(
            result,
            Err(SocksError::Io(std::io::ErrorKind::UnexpectedEof))
        );
    }
}
