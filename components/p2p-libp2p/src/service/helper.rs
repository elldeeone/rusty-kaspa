use std::net::SocketAddr;

use log::{debug, warn};
use tokio::io::{self, AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::{Duration, sleep, timeout};

use crate::helper_api::{HelperApi, HelperError};
use crate::transport::Libp2pError;

use triggered::Listener;

const HELPER_ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(200);
pub(super) const HELPER_MAX_LINE: usize = 8 * 1024;
const HELPER_READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
enum HelperReadError {
    Io(io::Error),
    TooLong,
    InvalidUtf8,
}

pub(super) fn validate_helper_listen_addr(addr: SocketAddr) -> Result<(), Libp2pError> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(Libp2pError::ListenFailed(format!(
            "helper API may only bind to a loopback address, got {addr}; use an SSH tunnel for remote access"
        )))
    }
}

pub(super) async fn start_helper_listener(
    addr: SocketAddr,
    api: HelperApi,
    mut shutdown: Option<Listener>,
) -> Result<(), Libp2pError> {
    validate_helper_listen_addr(addr)?;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| Libp2pError::ListenFailed(e.to_string()))?;
    log::info!("libp2p helper API listening on {addr}");

    tokio::spawn(async move {
        loop {
            if let Some(shutdown) = shutdown.as_mut() {
                tokio::select! {
                    _ = shutdown.clone() => {
                        debug!("libp2p helper listener shutting down");
                        break;
                    }
                    accept_res = listener.accept() => match accept_res {
                        Ok((stream, _)) => {
                            let api = api.clone();
                            tokio::spawn(async move {
                                handle_helper_connection(stream, api).await;
                            });
                        }
                        Err(err) => {
                            warn!("libp2p helper accept error: {err}");
                            sleep(HELPER_ACCEPT_RETRY_DELAY).await;
                        }
                    }
                }
            } else {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let api = api.clone();
                        tokio::spawn(async move {
                            handle_helper_connection(stream, api).await;
                        });
                    }
                    Err(err) => {
                        warn!("libp2p helper accept error: {err}");
                        sleep(HELPER_ACCEPT_RETRY_DELAY).await;
                    }
                }
            }
        }
    });

    Ok(())
}

async fn read_helper_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<String>, HelperReadError> {
    let mut bytes = Vec::with_capacity(HELPER_MAX_LINE.min(1024));
    loop {
        let available = reader.fill_buf().await.map_err(HelperReadError::Io)?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let chunk_len = newline.map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(chunk_len) > HELPER_MAX_LINE {
            return Err(HelperReadError::TooLong);
        }

        bytes.extend_from_slice(&available[..chunk_len]);
        reader.consume(chunk_len);
        if newline.is_some() {
            break;
        }
    }

    String::from_utf8(bytes).map(Some).map_err(|_| HelperReadError::InvalidUtf8)
}

pub(super) async fn handle_helper_connection(mut stream: tokio::net::TcpStream, api: HelperApi) {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let line = match timeout(HELPER_READ_TIMEOUT, read_helper_line(&mut reader)).await {
        Ok(Ok(Some(line))) => line,
        Ok(Ok(None)) => return,
        Ok(Err(HelperReadError::Io(err))) => {
            warn!("libp2p helper read error: {err}");
            return;
        }
        Ok(Err(HelperReadError::TooLong)) => {
            warn!("libp2p helper request exceeded max length ({} bytes)", HELPER_MAX_LINE);
            let _ = writer.write_all(br#"{"ok":false,"error":"request too long"}"#).await;
            let _ = writer.write_all(b"\n").await;
            return;
        }
        Ok(Err(HelperReadError::InvalidUtf8)) => {
            let resp = HelperApi::error_response(&HelperError::Invalid("invalid utf-8".into()));
            let _ = writer.write_all(resp.as_bytes()).await;
            let _ = writer.write_all(b"\n").await;
            return;
        }
        Err(_) => {
            warn!("libp2p helper read timeout after {:?}", HELPER_READ_TIMEOUT);
            let _ = writer.write_all(br#"{"ok":false,"error":"timeout waiting for request"}"#).await;
            let _ = writer.write_all(b"\n").await;
            return;
        }
    };

    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
    let resp_str = match api.handle_json(trimmed).await {
        Ok(r) => r,
        Err(e) => {
            warn!("libp2p helper request error: {e}");
            HelperApi::error_response(&e)
        }
    };
    let _ = writer.write_all(resp_str.as_bytes()).await;
    let _ = writer.write_all(b"\n").await;
}
