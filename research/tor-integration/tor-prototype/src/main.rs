use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use tor_interface::{
    legacy_tor_client::{LegacyTorClient, LegacyTorClientConfig, TorAuth},
    tor_crypto::{Ed25519PrivateKey, V3OnionServiceId},
    tor_provider::{OnionStream, TargetAddr, TorEvent, TorProvider},
};

#[derive(Parser, Debug)]
#[command(author, version, about = "Standalone prototype exercising Tor control, SOCKS, and onion service flows")]
struct Opts {
    /// Tor control port address (host:port)
    #[arg(long, default_value = "127.0.0.1:9051")]
    tor_control_addr: SocketAddr,

    /// Tor SOCKS listener address (host:port)
    #[arg(long, default_value = "127.0.0.1:9050")]
    tor_socks_addr: SocketAddr,

    /// Tor control password (overrides cookie authentication)
    #[arg(long)]
    tor_password: Option<String>,

    /// Tor control cookie file path
    #[arg(long)]
    tor_cookie: Option<PathBuf>,

    /// Seconds to wait for Tor bootstrap
    #[arg(long, default_value_t = 60)]
    bootstrap_timeout: u64,

    /// Optional clearnet host:port to fetch via Tor after bootstrap (e.g. "example.com:80")
    #[arg(long)]
    fetch: Option<String>,

    /// Timeout in seconds for the fetch request
    #[arg(long, default_value_t = 30)]
    fetch_timeout: u64,

    /// Enable stream isolation when performing the fetch
    #[arg(long, default_value_t = false)]
    isolate_fetch: bool,

    /// Publish a sample onion service on the given virtual port (e.g. 8080)
    #[arg(long)]
    publish_onion: Option<u16>,

    /// Seconds to keep the onion echo service alive
    #[arg(long, default_value_t = 120)]
    onion_runtime: u64,

    /// Load an existing onion private key from this path
    #[arg(long)]
    onion_key_in: Option<PathBuf>,

    /// Persist the generated onion private key to this path
    #[arg(long)]
    onion_key_out: Option<PathBuf>,
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    let auth = match (opts.tor_cookie.clone(), opts.tor_password.clone()) {
        (Some(cookie_path), _) => TorAuth::CookieFile(cookie_path),
        (None, Some(password)) => TorAuth::Password(password),
        (None, None) => TorAuth::Null,
    };

    let mut tor = LegacyTorClient::new(LegacyTorClientConfig::SystemTor {
        tor_socks_addr: opts.tor_socks_addr,
        tor_control_addr: opts.tor_control_addr,
        tor_control_auth: auth,
    })
    .with_context(|| format!("failed to connect to Tor control port at {}", opts.tor_control_addr))?;

    let version = tor.version();
    println!("Connected to Tor {version}");

    tor.bootstrap().context("failed to start Tor bootstrap")?;
    wait_for_bootstrap(&mut tor, Duration::from_secs(opts.bootstrap_timeout))?;

    println!("Tor bootstrap complete. SOCKS proxy assumed at {}", opts.tor_socks_addr);

    if let Some(target) = opts.fetch.as_deref() {
        let timeout = Duration::from_secs(opts.fetch_timeout);
        println!("Fetching {target} via Tor (timeout {:?}, isolation={})", timeout, opts.isolate_fetch);
        fetch_through_tor(&mut tor, target, timeout, opts.isolate_fetch)?;
    }

    if let Some(virt_port) = opts.publish_onion {
        let key = load_or_generate_key(opts.onion_key_in.as_deref())?;
        if let Some(path) = opts.onion_key_out.as_deref() {
            persist_key_if_missing(&key, path)?;
        }
        let runtime = Duration::from_secs(opts.onion_runtime);
        run_onion_echo_service(&mut tor, key, virt_port, runtime)?;
    }

    println!("Prototype run complete.");
    Ok(())
}

fn wait_for_bootstrap(tor: &mut LegacyTorClient, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut last_progress = 0;

    loop {
        for event in tor.update().context("processing tor events")? {
            match event {
                TorEvent::BootstrapStatus { progress, tag, summary } => {
                    if progress != last_progress {
                        println!("Bootstrap {progress}% – {tag}: {summary}");
                        last_progress = progress;
                    }
                }
                TorEvent::BootstrapComplete => return Ok(()),
                TorEvent::LogReceived { line } => println!("tor: {line}"),
                _ => {}
            }
        }

        if Instant::now() > deadline {
            bail!("timed out waiting for Tor bootstrap (>{:?})", timeout);
        }

        thread::sleep(Duration::from_millis(200));
    }
}

fn fetch_through_tor(tor: &mut LegacyTorClient, target: &str, timeout: Duration, isolate: bool) -> Result<()> {
    let target_addr = TargetAddr::from_str(target).with_context(|| format!("invalid target address '{target}'"))?;

    let host_header = match &target_addr {
        TargetAddr::Domain(domain) => domain.domain().to_string(),
        TargetAddr::Socket(socket) => socket.ip().to_string(),
        TargetAddr::OnionService(_) => {
            bail!("onion targets are not supported for --fetch")
        }
    };

    let token = isolate.then(|| tor.generate_token());
    let stream = tor.connect(target_addr.clone(), token).with_context(|| format!("tor connect to {target} failed"))?;

    if let Some(token) = token {
        tor.release_token(token);
    }

    let mut tcp: TcpStream = stream.into();
    tcp.set_read_timeout(Some(timeout)).context("setting read timeout")?;
    tcp.set_write_timeout(Some(timeout)).context("setting write timeout")?;

    let request = format!("GET / HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: kaspa-tor-prototype\r\nConnection: close\r\n\r\n");
    tcp.write_all(request.as_bytes()).context("sending HTTP request")?;

    let mut buffer = Vec::new();
    tcp.read_to_end(&mut buffer).context("reading response")?;

    let preview = String::from_utf8_lossy(&buffer);
    let preview = preview.lines().take(12).collect::<Vec<_>>().join("\n");
    println!("Fetch successful, response preview:\n{}\n", preview);

    Ok(())
}

fn run_onion_echo_service(tor: &mut LegacyTorClient, key: Ed25519PrivateKey, virt_port: u16, runtime: Duration) -> Result<()> {
    let onion_id = V3OnionServiceId::from_private_key(&key);
    let listener = tor.listener(&key, virt_port, None).context("failed to create onion service")?;
    listener.set_nonblocking(true).context("failed to configure onion listener")?;

    println!("Onion service readying at {}.onion:{} (runtime {:?})", onion_id, virt_port, runtime);

    let deadline = Instant::now() + runtime;
    let mut published = false;

    while Instant::now() < deadline {
        for event in tor.update().context("processing tor events")? {
            match event {
                TorEvent::OnionServicePublished { service_id } if service_id == onion_id => {
                    if !published {
                        println!("Onion service published: {}.onion:{}", service_id, virt_port);
                        published = true;
                    }
                }
                TorEvent::LogReceived { line } => println!("tor: {line}"),
                TorEvent::BootstrapStatus { .. } | TorEvent::BootstrapComplete => {}
                _ => {}
            }
        }

        match listener.accept() {
            Ok(Some(stream)) => {
                handle_inbound_connection(stream)?;
            }
            Ok(None) => {}
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(err) => return Err(err).context("error accepting onion connection"),
        }

        thread::sleep(Duration::from_millis(200));
    }

    println!("Shutting down onion service {}", onion_id);
    Ok(())
}

fn handle_inbound_connection(stream: OnionStream) -> Result<()> {
    let mut tcp: TcpStream = stream.into();
    tcp.set_read_timeout(Some(Duration::from_secs(10))).context("setting inbound read timeout")?;
    tcp.set_write_timeout(Some(Duration::from_secs(10))).context("setting inbound write timeout")?;

    let mut buf = [0u8; 1024];
    let bytes = tcp.read(&mut buf).context("reading inbound request")?;
    let lossless = String::from_utf8_lossy(&buf[..bytes]);
    let first_line = lossless.lines().next().unwrap_or("<binary>");
    println!("Incoming onion request: {first_line}");

    let body = format!("Hello from Kaspa Tor prototype!\nYou sent: {}\n", first_line);
    let response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}", body.len(), body);

    tcp.write_all(response.as_bytes()).context("sending onion response")?;
    Ok(())
}

fn load_or_generate_key(path: Option<&Path>) -> Result<Ed25519PrivateKey> {
    if let Some(path) = path {
        let blob = fs::read_to_string(path).with_context(|| format!("failed to read onion key from {}", path.display()))?;
        let key =
            Ed25519PrivateKey::from_key_blob(blob.trim()).with_context(|| format!("invalid onion key blob in {}", path.display()))?;
        println!("Loaded existing onion key from {}", path.display());
        Ok(key)
    } else {
        let key = Ed25519PrivateKey::generate();
        println!("Generated new onion private key");
        Ok(key)
    }
}

fn persist_key_if_missing(key: &Ed25519PrivateKey, path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    fs::write(path, key.to_key_blob()).with_context(|| format!("failed to write onion key to {}", path.display()))?;
    println!("Saved onion private key to {}", path.display());
    Ok(())
}
