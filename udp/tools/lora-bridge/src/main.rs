use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serialport::SerialPort;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEFAULT_BAUD: u32 = 9_600;
const LORA_APP_MTU: usize = 234;
const WAVESHARE_FIXED_PREFIX_LEN: usize = 6;
const WAVESHARE_RX_PREFIX_LEN: usize = 3;
const WAVESHARE_RX_STATUS_LEN: usize = 1;
const KUDP_MAGIC: &[u8; 4] = b"KUDP";
const FRAG_MAGIC: &[u8; 4] = b"KLR1";
const FRAG_HEADER_LEN: usize = 14;
const FRAG_CHUNK_LEN: usize = LORA_APP_MTU - FRAG_HEADER_LEN;

#[derive(Parser, Debug)]
#[command(name = "lora-bridge")]
#[command(about = "Bridge existing KUDP datagrams over Waveshare SX126X UART LoRa modules.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Read one or more KUDP datagrams and transmit them over a serial LoRa device.
    Tx(TxArgs),
    /// Receive KUDP datagrams from a serial LoRa device and write or forward them.
    Rx(RxArgs),
    /// Read the module configuration response in hardware config mode.
    ConfigRead(ConfigReadArgs),
}

#[derive(Args, Debug)]
struct SerialArgs {
    /// Serial device path, for example /dev/lora-left or /dev/lora-right.
    #[arg(long)]
    serial: PathBuf,

    /// UART baud rate. The lab Waveshare SX126X config uses 9600.
    #[arg(long, default_value_t = DEFAULT_BAUD)]
    baud: u32,

    /// Serial read timeout in milliseconds.
    #[arg(long, default_value_t = 250)]
    read_timeout_ms: u64,
}

#[derive(Args, Debug)]
struct TxArgs {
    #[command(flatten)]
    serial: SerialArgs,

    /// Input source for KUDP datagrams.
    #[arg(long, value_enum, default_value_t = InputKind::File)]
    input: InputKind,

    /// File containing exactly one KUDP datagram. Use "-" for stdin.
    #[arg(long, required_if_eq("input", "file"))]
    file: Option<PathBuf>,

    /// Local UDP socket to bind and read one datagram from.
    #[arg(long, required_if_eq("input", "udp"))]
    udp_bind: Option<SocketAddr>,

    /// Waveshare fixed-send 6-byte prefix as hex, default 000041000041.
    #[arg(long, default_value = "000041000041")]
    fixed_prefix_hex: String,

    /// Sleep this many milliseconds between LoRa writes.
    #[arg(long, default_value_t = 1500)]
    inter_frame_delay_ms: u64,

    /// Only allow single-packet datagrams; fail instead of fragmenting.
    #[arg(long)]
    no_fragment: bool,

    /// Bridge datagram id used in fragmented LoRa envelopes.
    #[arg(long, default_value_t = 1)]
    datagram_id: u32,
}

#[derive(Args, Debug)]
struct RxArgs {
    #[command(flatten)]
    serial: SerialArgs,

    /// Output sink for recovered KUDP datagrams.
    #[arg(long, value_enum, default_value_t = OutputKind::File)]
    output: OutputKind,

    /// File to write recovered bytes to. Use "-" for stdout.
    #[arg(long, required_if_eq("output", "file"))]
    file: Option<PathBuf>,

    /// Local UDP destination for recovered datagrams.
    #[arg(long, required_if_eq("output", "udp"))]
    udp_target: Option<SocketAddr>,

    /// Stop after this many recovered KUDP datagrams.
    #[arg(long, default_value_t = 1)]
    count: usize,

    /// Overall receive timeout in milliseconds.
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,

    /// Treat this many milliseconds without new serial bytes as one LoRa packet boundary.
    #[arg(long, default_value_t = 600)]
    packet_idle_ms: u64,
}

#[derive(Args, Debug)]
struct ConfigReadArgs {
    #[command(flatten)]
    serial: SerialArgs,

    /// Configuration read command. Default is c10009 for Waveshare/SX126X register read.
    #[arg(long, default_value = "c10009")]
    command_hex: String,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InputKind {
    File,
    Udp,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputKind {
    File,
    Stdout,
    Udp,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Tx(args) => tx(args),
        Command::Rx(args) => rx(args),
        Command::ConfigRead(args) => config_read(args),
    }
}

fn tx(args: TxArgs) -> Result<()> {
    let datagram = match args.input {
        InputKind::File => read_input_file(args.file.as_ref().context("--file is required")?)?,
        InputKind::Udp => read_udp_datagram(args.udp_bind.context("--udp-bind is required")?)?,
    };
    ensure_kudp(&datagram)?;

    let prefix = parse_fixed_prefix(&args.fixed_prefix_hex)?;
    let frames = fragment_datagram(&datagram, args.datagram_id, args.no_fragment)?;
    let mut port = open_serial(&args.serial)?;
    let delay = Duration::from_millis(args.inter_frame_delay_ms);

    for (idx, frame) in frames.iter().enumerate() {
        if frame.len() > LORA_APP_MTU {
            bail!("internal frame too large: {} > {}", frame.len(), LORA_APP_MTU);
        }
        let mut wire = Vec::with_capacity(WAVESHARE_FIXED_PREFIX_LEN + frame.len());
        wire.extend_from_slice(&prefix);
        wire.extend_from_slice(frame);
        port.write_all(&wire).context("write serial")?;
        port.flush().context("flush serial")?;
        eprintln!("sent frame {}/{}: app_payload={} serial_bytes={}", idx + 1, frames.len(), frame.len(), wire.len());
        if idx + 1 < frames.len() && !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }

    Ok(())
}

fn rx(args: RxArgs) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be greater than zero");
    }

    let mut port = open_serial(&args.serial)?;
    let deadline = Instant::now() + Duration::from_millis(args.timeout_ms);
    let packet_idle = Duration::from_millis(args.packet_idle_ms);
    let mut reassembly = Reassembler::default();
    let mut recovered = 0usize;

    while recovered < args.count {
        let raw = read_lora_packet(&mut *port, deadline, packet_idle).context("read LoRa packet")?;
        let app_payload = strip_waveshare_rx(&raw)?;
        if let Some(datagram) = reassembly.push(app_payload)? {
            ensure_kudp(&datagram)?;
            write_output(&args, &datagram)?;
            recovered += 1;
            eprintln!("recovered KUDP datagram {}/{}: {} bytes", recovered, args.count, datagram.len());
        }
    }

    Ok(())
}

fn config_read(args: ConfigReadArgs) -> Result<()> {
    let command = parse_hex(&args.command_hex)?;
    let mut port = open_serial(&args.serial)?;
    port.write_all(&command).context("write config command")?;
    port.flush().context("flush config command")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let packet = read_lora_packet(&mut *port, deadline, Duration::from_millis(args.serial.read_timeout_ms))?;
    println!("{}", to_hex_spaced(&packet));
    Ok(())
}

fn open_serial(args: &SerialArgs) -> Result<Box<dyn SerialPort>> {
    serialport::new(args.serial.to_string_lossy(), args.baud)
        .timeout(Duration::from_millis(args.read_timeout_ms))
        .open()
        .with_context(|| format!("open serial {}", args.serial.display()))
}

fn read_input_file(path: &PathBuf) -> Result<Vec<u8>> {
    if path.as_os_str() == "-" {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf).context("read stdin")?;
        Ok(buf)
    } else {
        fs::read(path).with_context(|| format!("read {}", path.display()))
    }
}

fn read_udp_datagram(bind: SocketAddr) -> Result<Vec<u8>> {
    let socket = UdpSocket::bind(bind).with_context(|| format!("bind UDP {bind}"))?;
    let mut buf = vec![0u8; 65_535];
    let (len, peer) = socket.recv_from(&mut buf).context("receive UDP datagram")?;
    buf.truncate(len);
    eprintln!("received UDP datagram from {peer}: {len} bytes");
    Ok(buf)
}

fn write_output(args: &RxArgs, datagram: &[u8]) -> Result<()> {
    match args.output {
        OutputKind::File => {
            let path = args.file.as_ref().context("--file is required")?;
            if path.as_os_str() == "-" {
                io::stdout().write_all(datagram).context("write stdout")?;
            } else {
                fs::write(path, datagram).with_context(|| format!("write {}", path.display()))?;
            }
        }
        OutputKind::Stdout => {
            io::stdout().write_all(datagram).context("write stdout")?;
        }
        OutputKind::Udp => {
            let target = args.udp_target.context("--udp-target is required")?;
            let socket = UdpSocket::bind("127.0.0.1:0").context("bind UDP output socket")?;
            socket.send_to(datagram, target).with_context(|| format!("send UDP to {target}"))?;
        }
    }
    Ok(())
}

fn fragment_datagram(datagram: &[u8], datagram_id: u32, no_fragment: bool) -> Result<Vec<Vec<u8>>> {
    ensure_kudp(datagram)?;
    if datagram.len() <= LORA_APP_MTU {
        return Ok(vec![datagram.to_vec()]);
    }
    if no_fragment {
        bail!("datagram is {} bytes, larger than LoRa MTU {}", datagram.len(), LORA_APP_MTU);
    }
    if datagram.len() > u16::MAX as usize {
        bail!("datagram is too large for MVP bridge envelope: {} bytes", datagram.len());
    }

    let frag_cnt = datagram.len().div_ceil(FRAG_CHUNK_LEN);
    if frag_cnt > u16::MAX as usize {
        bail!("too many fragments: {frag_cnt}");
    }

    let mut frames = Vec::with_capacity(frag_cnt);
    for (frag_ix, chunk) in datagram.chunks(FRAG_CHUNK_LEN).enumerate() {
        let mut frame = Vec::with_capacity(FRAG_HEADER_LEN + chunk.len());
        frame.extend_from_slice(FRAG_MAGIC);
        frame.extend_from_slice(&datagram_id.to_le_bytes());
        frame.extend_from_slice(&(frag_ix as u16).to_le_bytes());
        frame.extend_from_slice(&(frag_cnt as u16).to_le_bytes());
        frame.extend_from_slice(&(datagram.len() as u16).to_le_bytes());
        frame.extend_from_slice(chunk);
        frames.push(frame);
    }
    Ok(frames)
}

#[derive(Default)]
struct Reassembler {
    pending: BTreeMap<u32, PendingDatagram>,
}

impl Reassembler {
    fn push(&mut self, app_payload: &[u8]) -> Result<Option<Vec<u8>>> {
        if app_payload.starts_with(KUDP_MAGIC) {
            return Ok(Some(app_payload.to_vec()));
        }
        if !app_payload.starts_with(FRAG_MAGIC) {
            bail!("received payload is neither raw KUDP nor lora-bridge fragment");
        }
        if app_payload.len() < FRAG_HEADER_LEN {
            bail!("fragment is too short: {} bytes", app_payload.len());
        }

        let datagram_id = u32::from_le_bytes(app_payload[4..8].try_into().unwrap());
        let frag_ix = u16::from_le_bytes(app_payload[8..10].try_into().unwrap()) as usize;
        let frag_cnt = u16::from_le_bytes(app_payload[10..12].try_into().unwrap()) as usize;
        let total_len = u16::from_le_bytes(app_payload[12..14].try_into().unwrap()) as usize;
        let chunk = &app_payload[FRAG_HEADER_LEN..];

        if frag_cnt == 0 {
            bail!("fragment count is zero");
        }
        if frag_ix >= frag_cnt {
            bail!("fragment index {frag_ix} out of range {frag_cnt}");
        }
        if total_len == 0 {
            bail!("fragment total length is zero");
        }
        if chunk.is_empty() {
            bail!("fragment payload is empty");
        }

        let pending = self.pending.entry(datagram_id).or_insert_with(|| PendingDatagram::new(frag_cnt, total_len));
        pending.insert(frag_ix, frag_cnt, total_len, chunk)?;

        if pending.is_complete() {
            let pending = self.pending.remove(&datagram_id).unwrap();
            return Ok(Some(pending.assemble()?));
        }

        eprintln!("received fragment {}/{} for datagram {}", frag_ix + 1, frag_cnt, datagram_id);
        Ok(None)
    }
}

struct PendingDatagram {
    frag_cnt: usize,
    total_len: usize,
    fragments: Vec<Option<Vec<u8>>>,
}

impl PendingDatagram {
    fn new(frag_cnt: usize, total_len: usize) -> Self {
        Self { frag_cnt, total_len, fragments: vec![None; frag_cnt] }
    }

    fn insert(&mut self, frag_ix: usize, frag_cnt: usize, total_len: usize, chunk: &[u8]) -> Result<()> {
        if self.frag_cnt != frag_cnt || self.total_len != total_len {
            bail!("fragment metadata changed within datagram");
        }
        self.fragments[frag_ix] = Some(chunk.to_vec());
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.fragments.iter().all(Option::is_some)
    }

    fn assemble(self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(self.total_len);
        for fragment in self.fragments {
            out.extend_from_slice(&fragment.context("missing fragment")?);
        }
        if out.len() != self.total_len {
            bail!("reassembled length mismatch: got {}, expected {}", out.len(), self.total_len);
        }
        Ok(out)
    }
}

fn strip_waveshare_rx(raw: &[u8]) -> Result<&[u8]> {
    if raw.len() < WAVESHARE_RX_PREFIX_LEN + WAVESHARE_RX_STATUS_LEN {
        bail!("LoRa RX packet too short: {} bytes", raw.len());
    }
    Ok(&raw[WAVESHARE_RX_PREFIX_LEN..raw.len() - WAVESHARE_RX_STATUS_LEN])
}

fn read_lora_packet(port: &mut dyn SerialPort, deadline: Instant, packet_idle: Duration) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 512];
    let mut last_byte_at: Option<Instant> = None;

    loop {
        if Instant::now() >= deadline {
            if out.is_empty() {
                bail!("timed out waiting for LoRa packet");
            }
            break;
        }

        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                last_byte_at = Some(Instant::now());
            }
            Err(err) if err.kind() == io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err).context("read serial"),
        }

        if let Some(last) = last_byte_at {
            if !out.is_empty() && last.elapsed() >= packet_idle {
                break;
            }
        }
    }

    Ok(out)
}

fn ensure_kudp(datagram: &[u8]) -> Result<()> {
    if !datagram.starts_with(KUDP_MAGIC) {
        bail!("input does not start with KUDP magic");
    }
    Ok(())
}

fn parse_fixed_prefix(hex: &str) -> Result<[u8; WAVESHARE_FIXED_PREFIX_LEN]> {
    let bytes = parse_hex(hex)?;
    if bytes.len() != WAVESHARE_FIXED_PREFIX_LEN {
        bail!("fixed prefix must be exactly 6 bytes, got {}", bytes.len());
    }
    let mut out = [0u8; WAVESHARE_FIXED_PREFIX_LEN];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn parse_hex(input: &str) -> Result<Vec<u8>> {
    let compact: String = input.chars().filter(|c| !c.is_ascii_whitespace() && *c != ':' && *c != '_').collect();
    if compact.len() % 2 != 0 {
        bail!("hex string has odd length");
    }
    let mut out = Vec::with_capacity(compact.len() / 2);
    for pair in compact.as_bytes().chunks_exact(2) {
        let s = std::str::from_utf8(pair).unwrap();
        out.push(u8::from_str_radix(s, 16).with_context(|| format!("invalid hex byte {s}"))?);
    }
    Ok(out)
}

fn to_hex_spaced(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kudp_datagram(len: usize) -> Vec<u8> {
        assert!(len >= 4);
        let mut bytes = vec![0xAA; len];
        bytes[..4].copy_from_slice(KUDP_MAGIC);
        bytes
    }

    #[test]
    fn raw_delta_sized_datagram_is_single_packet() {
        let datagram = kudp_datagram(200);
        let frames = fragment_datagram(&datagram, 99, false).unwrap();
        assert_eq!(frames, vec![datagram]);
    }

    #[test]
    fn snapshot_sized_datagram_fragments_and_reassembles() {
        let datagram = kudp_datagram(329);
        let frames = fragment_datagram(&datagram, 7, false).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|frame| frame.len() <= LORA_APP_MTU));

        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(&frames[0]).unwrap().is_none());
        let out = reassembler.push(&frames[1]).unwrap().unwrap();
        assert_eq!(out, datagram);
    }

    #[test]
    fn no_fragment_rejects_oversized_datagram() {
        let datagram = kudp_datagram(329);
        assert!(fragment_datagram(&datagram, 7, true).is_err());
    }

    #[test]
    fn strips_waveshare_rx_prefix_and_status() {
        let datagram = kudp_datagram(16);
        let mut raw = vec![0x00, 0x00, 0x41];
        raw.extend_from_slice(&datagram);
        raw.push(0x80);
        assert_eq!(strip_waveshare_rx(&raw).unwrap(), datagram);
    }

    #[test]
    fn fixed_prefix_accepts_spaced_hex() {
        assert_eq!(parse_fixed_prefix("00 00 41 00 00 41").unwrap(), [0x00, 0x00, 0x41, 0x00, 0x00, 0x41]);
    }
}
