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
const RELIABLE_FRAG_MAGIC: &[u8; 4] = b"KLR2";
const ACK_MAGIC: &[u8; 4] = b"KLA1";
const FRAG_HEADER_LEN: usize = 14;
const RELIABLE_FRAG_HEADER_LEN: usize = 18;
const ACK_FRAME_LEN: usize = 14;
const FRAG_CHUNK_LEN: usize = LORA_APP_MTU - FRAG_HEADER_LEN;
const RELIABLE_FRAG_CHUNK_LEN: usize = LORA_APP_MTU - RELIABLE_FRAG_HEADER_LEN;

#[derive(Parser, Debug)]
#[command(name = "lora-bridge")]
#[command(about = "Bridge existing KUDP datagrams over Waveshare SX126X UART LoRa modules without changing KUDP bytes.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Read KUDP datagrams from a file or UDP socket and transmit them over a serial LoRa device.
    Tx(TxArgs),
    /// Receive KUDP datagrams from a serial LoRa device and write them to a file/stdout/UDP socket.
    Rx(RxArgs),
    /// Read the module configuration response in hardware config mode.
    ConfigRead(ConfigReadArgs),
}

#[derive(Args, Debug)]
struct SerialArgs {
    /// Serial device path, for example /dev/lora-left or /dev/lora-right. Wrong device or jumper mode usually shows up as open/read timeout errors.
    #[arg(long)]
    serial: PathBuf,

    /// UART baud rate. The tested Waveshare SX126X config uses 9600.
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

    /// Local UDP socket to bind and read KUDP datagrams from. Use --count for multi-datagram live-producer runs.
    #[arg(long, required_if_eq("input", "udp"))]
    udp_bind: Option<SocketAddr>,

    /// Waveshare fixed-send 6-byte prefix as hex: dest_hi dest_lo freq src_hi src_lo src_freq.
    #[arg(long, default_value = "000041000041")]
    fixed_prefix_hex: String,

    /// Sleep this many milliseconds between serial writes. 1500 ms is the tested minimum; 2500 ms was used for live runs.
    #[arg(long)]
    inter_frame_delay_ms: Option<u64>,

    /// Enable bridge-local ACK/retry for fragmented datagrams. Raw one-packet deltas are still sent once unless --reliable-all is set.
    #[arg(long)]
    reliable_fragments: bool,

    /// Wrap all KUDP datagrams in the reliable envelope, including single-packet deltas.
    #[arg(long)]
    reliable_all: bool,

    /// Retry count per fragment when --reliable-fragments is enabled.
    #[arg(long, default_value_t = 4)]
    retry_count: usize,

    /// ACK timeout in milliseconds when --reliable-fragments is enabled.
    #[arg(long, default_value_t = 3_000)]
    ack_timeout_ms: u64,

    /// Bridge-local session id for reliable fragment ACKs.
    #[arg(long, default_value_t = 1)]
    session_id: u32,

    /// Alias for --session-id when multiple bridge groups share the same RF channel.
    #[arg(long)]
    group_id: Option<u32>,

    /// Only allow single-packet datagrams; fail instead of fragmenting.
    #[arg(long)]
    no_fragment: bool,

    /// Bridge datagram id used in fragmented LoRa envelopes. Incremented automatically for each datagram in a multi-count TX run.
    #[arg(long, default_value_t = 1)]
    datagram_id: u32,

    /// Number of datagrams to read from the input source and send. File input always sends the file once.
    #[arg(long, default_value_t = 1)]
    count: usize,
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

    /// Overall receive timeout in milliseconds. If fragments are pending, timeout errors include the pending fragment summary.
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,

    /// Treat this many milliseconds without new serial bytes as one LoRa packet boundary.
    #[arg(long, default_value_t = 600)]
    packet_idle_ms: u64,

    /// Waveshare fixed-send 6-byte prefix used for bridge-local ACKs.
    #[arg(long, default_value = "000041000041")]
    fixed_prefix_hex: String,

    /// Send ACKs for reliable bridge fragments.
    #[arg(long, default_value_t = true)]
    ack_fragments: bool,

    /// Expected bridge-local session id for reliable fragment ACKs.
    #[arg(long, default_value_t = 1)]
    session_id: u32,

    /// Alias for --session-id when multiple bridge groups share the same RF channel.
    #[arg(long)]
    group_id: Option<u32>,
}

#[derive(Args, Debug)]
struct ConfigReadArgs {
    #[command(flatten)]
    serial: SerialArgs,

    /// Configuration read command. Default is c10009 for Waveshare/SX126X register read.
    #[arg(long, default_value = "c10009")]
    command_hex: String,
}

impl TxArgs {
    fn effective_session_id(&self) -> u32 {
        self.group_id.unwrap_or(self.session_id)
    }
}

impl RxArgs {
    fn effective_session_id(&self) -> u32 {
        self.group_id.unwrap_or(self.session_id)
    }
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
    let prefix = parse_fixed_prefix(&args.fixed_prefix_hex)?;
    let datagrams = read_input_datagrams(&args)?;
    let mut port = open_serial(&args.serial)?;
    let mut stats = BridgeStats::default();
    let started_at = Instant::now();
    let session_id = args.effective_session_id();

    for (datagram_idx, datagram) in datagrams.iter().enumerate() {
        let datagram_id = args.datagram_id.wrapping_add(datagram_idx as u32);
        let frames = fragment_datagram_with_options(
            datagram,
            datagram_id,
            args.no_fragment,
            args.reliable_fragments || args.reliable_all,
            args.reliable_all,
            session_id,
        )?;
        let delay = Duration::from_millis(args.inter_frame_delay_ms.unwrap_or_else(|| adaptive_delay_ms(frames.len())));
        for (idx, frame) in frames.iter().enumerate() {
            if frame.len() > LORA_APP_MTU {
                bail!("internal frame too large: {} > {}", frame.len(), LORA_APP_MTU);
            }
            let frag_ix = fragment_index(frame).unwrap_or(idx as u16);
            let (attempts, serial_bytes) = if (args.reliable_fragments || args.reliable_all) && is_reliable_fragment(frame) {
                write_reliable_frame(&mut *port, &prefix, frame, session_id, datagram_id, frag_ix, &args, &mut stats)?
            } else {
                let serial_bytes = write_lora_app_payload(&mut *port, &prefix, frame)?;
                (1, serial_bytes)
            };
            stats.fragments_sent += attempts;
            eprintln!(
                "sent datagram {}/{} datagram_id={} session_id={} frame {}/{} frag_ix={}: app_payload={} serial_bytes={} elapsed_ms={}",
                datagram_idx + 1,
                datagrams.len(),
                datagram_id,
                session_id,
                idx + 1,
                frames.len(),
                frag_ix,
                frame.len(),
                serial_bytes,
                started_at.elapsed().as_millis()
            );
            if (datagram_idx + 1 < datagrams.len() || idx + 1 < frames.len()) && !delay.is_zero() {
                std::thread::sleep(delay);
            }
        }
        stats.datagrams_sent += 1;
        stats.bytes_sent += datagram.len();
    }

    stats.print("tx", started_at);
    Ok(())
}

fn rx(args: RxArgs) -> Result<()> {
    if args.count == 0 {
        bail!("--count must be greater than zero");
    }

    let mut port = open_serial(&args.serial)?;
    let ack_prefix = parse_fixed_prefix(&args.fixed_prefix_hex)?;
    let deadline = Instant::now() + Duration::from_millis(args.timeout_ms);
    let packet_idle = Duration::from_millis(args.packet_idle_ms);
    let mut reassembly = Reassembler::default();
    let mut recovered = 0usize;
    let mut stats = BridgeStats::default();
    let started_at = Instant::now();
    let session_id = args.effective_session_id();

    while recovered < args.count {
        let raw = match read_lora_packet(&mut *port, deadline, packet_idle) {
            Ok(raw) => raw,
            Err(err) if reassembly.has_pending() => {
                stats.receive_timeouts += 1;
                stats.print("rx", started_at);
                bail!("read LoRa packet failed with pending fragments: {}; cause: {err}", reassembly.pending_summary())
            }
            Err(err) => {
                stats.receive_timeouts += 1;
                stats.print("rx", started_at);
                return Err(err).context("read LoRa packet");
            }
        };
        let app_payload = match strip_waveshare_rx(&raw) {
            Ok(app_payload) => app_payload,
            Err(err) => {
                stats.corrupt_frames += 1;
                stats.print("rx", started_at);
                return Err(err);
            }
        };
        if app_payload.starts_with(ACK_MAGIC) {
            continue;
        }
        match ack_for_payload(app_payload, session_id) {
            Ok(Some(ack)) => {
                if args.ack_fragments {
                    write_lora_app_payload(&mut *port, &ack_prefix, &ack)?;
                    stats.acks_sent += 1;
                }
            }
            Ok(None) => {}
            Err(err) => {
                stats.corrupt_frames += 1;
                stats.print("rx", started_at);
                return Err(err);
            }
        }
        match reassembly.push_with_stats(app_payload, &mut stats) {
            Ok(Some(datagram)) => {
                ensure_kudp(&datagram)?;
                write_output(&args, &datagram)?;
                recovered += 1;
                stats.datagrams_recovered += 1;
                stats.bytes_recovered += datagram.len();
                eprintln!(
                    "recovered KUDP datagram {}/{}: {} bytes session_id={} elapsed_ms={}",
                    recovered,
                    args.count,
                    datagram.len(),
                    session_id,
                    started_at.elapsed().as_millis()
                );
            }
            Ok(None) => {}
            Err(err) => {
                stats.reassembly_failures += 1;
                stats.print("rx", started_at);
                return Err(err);
            }
        }
    }

    stats.print("rx", started_at);
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

fn read_input_datagrams(args: &TxArgs) -> Result<Vec<Vec<u8>>> {
    if args.count == 0 {
        bail!("--count must be greater than zero");
    }
    match args.input {
        InputKind::File => {
            let datagram = read_input_file(args.file.as_ref().context("--file is required")?)?;
            ensure_kudp(&datagram)?;
            Ok(vec![datagram])
        }
        InputKind::Udp => read_udp_datagrams(args.udp_bind.context("--udp-bind is required")?, args.count),
    }
}

fn read_udp_datagrams(bind: SocketAddr, count: usize) -> Result<Vec<Vec<u8>>> {
    let socket = UdpSocket::bind(bind).with_context(|| format!("bind UDP {bind}"))?;
    let mut datagrams = Vec::with_capacity(count);
    for idx in 0..count {
        let mut buf = vec![0u8; 65_535];
        let (len, peer) = socket.recv_from(&mut buf).context("receive UDP datagram")?;
        buf.truncate(len);
        ensure_kudp(&buf)?;
        eprintln!("received UDP datagram {}/{} from {peer}: {len} bytes", idx + 1, count);
        datagrams.push(buf);
    }
    Ok(datagrams)
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

fn write_lora_app_payload(port: &mut dyn SerialPort, prefix: &[u8; WAVESHARE_FIXED_PREFIX_LEN], app_payload: &[u8]) -> Result<usize> {
    let mut wire = Vec::with_capacity(WAVESHARE_FIXED_PREFIX_LEN + app_payload.len());
    wire.extend_from_slice(prefix);
    wire.extend_from_slice(app_payload);
    port.write_all(&wire).context("write serial")?;
    port.flush().context("flush serial")?;
    Ok(wire.len())
}

fn write_reliable_frame(
    port: &mut dyn SerialPort,
    prefix: &[u8; WAVESHARE_FIXED_PREFIX_LEN],
    frame: &[u8],
    session_id: u32,
    datagram_id: u32,
    frag_ix: u16,
    args: &TxArgs,
    stats: &mut BridgeStats,
) -> Result<(usize, usize)> {
    let mut attempts = 0usize;
    for attempt in 0..=args.retry_count {
        attempts += 1;
        let serial_bytes = write_lora_app_payload(port, prefix, frame)?;
        if wait_for_ack(
            port,
            Duration::from_millis(args.ack_timeout_ms),
            Duration::from_millis(args.serial.read_timeout_ms),
            session_id,
            datagram_id,
            frag_ix,
        )? {
            if attempt > 0 {
                eprintln!(
                    "ack received after retry attempt={attempt} session_id={session_id} datagram_id={datagram_id} frag_ix={frag_ix}"
                );
            }
            return Ok((attempts, serial_bytes));
        }
        if should_retry(attempt, args.retry_count) {
            stats.retries += 1;
            stats.missing_fragments += 1;
            eprintln!(
                "ack timeout; retrying session_id={session_id} datagram_id={datagram_id} frag_ix={frag_ix} attempt={}",
                attempt + 1
            );
        }
    }
    bail!("retry exhausted for session_id={session_id} datagram_id={datagram_id} frag_ix={frag_ix}");
}

fn wait_for_ack(
    port: &mut dyn SerialPort,
    timeout: Duration,
    packet_idle: Duration,
    session_id: u32,
    datagram_id: u32,
    frag_ix: u16,
) -> Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        let raw = match read_lora_packet(port, deadline, packet_idle) {
            Ok(raw) => raw,
            Err(err) if err.to_string().contains("timed out") => return Ok(false),
            Err(err) => return Err(err),
        };
        let app_payload = strip_waveshare_rx(&raw)?;
        if let Some(ack) = parse_ack(app_payload)? {
            if ack.session_id == session_id && ack.datagram_id == datagram_id && ack.frag_ix == frag_ix {
                return Ok(true);
            }
        }
    }
}

fn adaptive_delay_ms(frame_count: usize) -> u64 {
    if frame_count <= 1 {
        250
    } else {
        1_500
    }
}

fn should_retry(attempt: usize, retry_count: usize) -> bool {
    attempt < retry_count
}

#[cfg(test)]
fn fragment_datagram(datagram: &[u8], datagram_id: u32, no_fragment: bool) -> Result<Vec<Vec<u8>>> {
    fragment_datagram_with_options(datagram, datagram_id, no_fragment, false, false, 0)
}

fn fragment_datagram_with_options(
    datagram: &[u8],
    datagram_id: u32,
    no_fragment: bool,
    reliable: bool,
    reliable_all: bool,
    session_id: u32,
) -> Result<Vec<Vec<u8>>> {
    ensure_kudp(datagram)?;
    if datagram.len() <= LORA_APP_MTU && !reliable_all {
        return Ok(vec![datagram.to_vec()]);
    }
    if no_fragment {
        bail!("datagram is {} bytes, larger than LoRa MTU {}", datagram.len(), LORA_APP_MTU);
    }
    if datagram.len() > u16::MAX as usize {
        bail!("datagram is too large for MVP bridge envelope: {} bytes", datagram.len());
    }

    let header_len = if reliable { RELIABLE_FRAG_HEADER_LEN } else { FRAG_HEADER_LEN };
    let chunk_len = if reliable { RELIABLE_FRAG_CHUNK_LEN } else { FRAG_CHUNK_LEN };
    let frag_cnt = datagram.len().div_ceil(chunk_len);
    if frag_cnt > u16::MAX as usize {
        bail!("too many fragments: {frag_cnt}");
    }

    let mut frames = Vec::with_capacity(frag_cnt);
    for (frag_ix, chunk) in datagram.chunks(chunk_len).enumerate() {
        let mut frame = Vec::with_capacity(header_len + chunk.len());
        frame.extend_from_slice(if reliable { RELIABLE_FRAG_MAGIC } else { FRAG_MAGIC });
        if reliable {
            frame.extend_from_slice(&session_id.to_le_bytes());
        }
        frame.extend_from_slice(&datagram_id.to_le_bytes());
        frame.extend_from_slice(&(frag_ix as u16).to_le_bytes());
        frame.extend_from_slice(&(frag_cnt as u16).to_le_bytes());
        frame.extend_from_slice(&(datagram.len() as u16).to_le_bytes());
        frame.extend_from_slice(chunk);
        frames.push(frame);
    }
    Ok(frames)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AckFrame {
    session_id: u32,
    datagram_id: u32,
    frag_ix: u16,
}

fn make_ack(session_id: u32, datagram_id: u32, frag_ix: u16) -> Vec<u8> {
    let mut frame = Vec::with_capacity(ACK_FRAME_LEN);
    frame.extend_from_slice(ACK_MAGIC);
    frame.extend_from_slice(&session_id.to_le_bytes());
    frame.extend_from_slice(&datagram_id.to_le_bytes());
    frame.extend_from_slice(&frag_ix.to_le_bytes());
    frame
}

fn parse_ack(app_payload: &[u8]) -> Result<Option<AckFrame>> {
    if !app_payload.starts_with(ACK_MAGIC) {
        return Ok(None);
    }
    if app_payload.len() != ACK_FRAME_LEN {
        bail!("ACK frame has wrong length: {}", app_payload.len());
    }
    Ok(Some(AckFrame {
        session_id: u32::from_le_bytes(app_payload[4..8].try_into().unwrap()),
        datagram_id: u32::from_le_bytes(app_payload[8..12].try_into().unwrap()),
        frag_ix: u16::from_le_bytes(app_payload[12..14].try_into().unwrap()),
    }))
}

fn is_reliable_fragment(app_payload: &[u8]) -> bool {
    app_payload.starts_with(RELIABLE_FRAG_MAGIC)
}

fn fragment_index(app_payload: &[u8]) -> Option<u16> {
    if app_payload.starts_with(RELIABLE_FRAG_MAGIC) && app_payload.len() >= RELIABLE_FRAG_HEADER_LEN {
        return Some(u16::from_le_bytes(app_payload[12..14].try_into().unwrap()));
    }
    if app_payload.starts_with(FRAG_MAGIC) && app_payload.len() >= FRAG_HEADER_LEN {
        return Some(u16::from_le_bytes(app_payload[8..10].try_into().unwrap()));
    }
    None
}

fn ack_for_payload(app_payload: &[u8], expected_session_id: u32) -> Result<Option<Vec<u8>>> {
    if !app_payload.starts_with(RELIABLE_FRAG_MAGIC) {
        return Ok(None);
    }
    let fragment = parse_fragment(app_payload)?;
    if fragment.session_id != Some(expected_session_id) {
        return Ok(None);
    }
    Ok(Some(make_ack(expected_session_id, fragment.datagram_id, fragment.frag_ix as u16)))
}

#[derive(Default)]
struct BridgeStats {
    datagrams_sent: usize,
    datagrams_recovered: usize,
    fragments_sent: usize,
    fragments_received: usize,
    retries: usize,
    duplicate_fragments: usize,
    missing_fragments: usize,
    corrupt_frames: usize,
    receive_timeouts: usize,
    reassembly_failures: usize,
    acks_sent: usize,
    bytes_sent: usize,
    bytes_recovered: usize,
}

impl BridgeStats {
    fn print(&self, role: &str, started_at: Instant) {
        let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
        let datagrams = self.datagrams_sent + self.datagrams_recovered;
        let bytes = self.bytes_sent + self.bytes_recovered;
        eprintln!(
            "bridge_summary role={role} datagrams_sent={} datagrams_recovered={} fragments_sent={} fragments_received={} retries={} duplicate_fragments={} missing_fragments={} corrupt_frames={} receive_timeouts={} reassembly_failures={} acks_sent={} datagrams_per_minute={:.2} bytes_per_minute={:.0}",
            self.datagrams_sent,
            self.datagrams_recovered,
            self.fragments_sent,
            self.fragments_received,
            self.retries,
            self.duplicate_fragments,
            self.missing_fragments,
            self.corrupt_frames,
            self.receive_timeouts,
            self.reassembly_failures,
            self.acks_sent,
            datagrams as f64 * 60.0 / elapsed,
            bytes as f64 * 60.0 / elapsed
        );
    }
}

#[derive(Debug)]
struct Fragment<'a> {
    session_id: Option<u32>,
    datagram_id: u32,
    frag_ix: usize,
    frag_cnt: usize,
    total_len: usize,
    chunk: &'a [u8],
}

fn parse_fragment(app_payload: &[u8]) -> Result<Fragment<'_>> {
    let (session_id, header_len, offset) = if app_payload.starts_with(RELIABLE_FRAG_MAGIC) {
        if app_payload.len() < RELIABLE_FRAG_HEADER_LEN {
            bail!("reliable fragment is too short: {} bytes", app_payload.len());
        }
        (Some(u32::from_le_bytes(app_payload[4..8].try_into().unwrap())), RELIABLE_FRAG_HEADER_LEN, 8)
    } else if app_payload.starts_with(FRAG_MAGIC) {
        if app_payload.len() < FRAG_HEADER_LEN {
            bail!("fragment is too short: {} bytes", app_payload.len());
        }
        (None, FRAG_HEADER_LEN, 4)
    } else {
        bail!("received payload is neither raw KUDP nor lora-bridge fragment");
    };

    let datagram_id = u32::from_le_bytes(app_payload[offset..offset + 4].try_into().unwrap());
    let frag_ix = u16::from_le_bytes(app_payload[offset + 4..offset + 6].try_into().unwrap()) as usize;
    let frag_cnt = u16::from_le_bytes(app_payload[offset + 6..offset + 8].try_into().unwrap()) as usize;
    let total_len = u16::from_le_bytes(app_payload[offset + 8..offset + 10].try_into().unwrap()) as usize;
    let chunk = &app_payload[header_len..];

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

    Ok(Fragment { session_id, datagram_id, frag_ix, frag_cnt, total_len, chunk })
}

#[derive(Default)]
struct Reassembler {
    pending: BTreeMap<u32, PendingDatagram>,
}

impl Reassembler {
    #[cfg(test)]
    fn push(&mut self, app_payload: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut stats = BridgeStats::default();
        self.push_with_stats(app_payload, &mut stats)
    }

    fn push_with_stats(&mut self, app_payload: &[u8], stats: &mut BridgeStats) -> Result<Option<Vec<u8>>> {
        if app_payload.starts_with(KUDP_MAGIC) {
            return Ok(Some(app_payload.to_vec()));
        }
        let fragment = parse_fragment(app_payload)?;
        stats.fragments_received += 1;
        let pending =
            self.pending.entry(fragment.datagram_id).or_insert_with(|| PendingDatagram::new(fragment.frag_cnt, fragment.total_len));
        if pending.insert(fragment.frag_ix, fragment.frag_cnt, fragment.total_len, fragment.chunk)? {
            stats.duplicate_fragments += 1;
        }

        if pending.is_complete() {
            let pending = self.pending.remove(&fragment.datagram_id).unwrap();
            return Ok(Some(pending.assemble()?));
        }

        eprintln!("received fragment {}/{} for datagram {}", fragment.frag_ix + 1, fragment.frag_cnt, fragment.datagram_id);
        Ok(None)
    }

    fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn pending_summary(&self) -> String {
        if self.pending.is_empty() {
            return "none".to_string();
        }
        self.pending
            .iter()
            .map(|(id, pending)| {
                format!("datagram_id={id} received={}/{} total_len={}", pending.received_count(), pending.frag_cnt, pending.total_len)
            })
            .collect::<Vec<_>>()
            .join("; ")
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

    fn insert(&mut self, frag_ix: usize, frag_cnt: usize, total_len: usize, chunk: &[u8]) -> Result<bool> {
        if self.frag_cnt != frag_cnt || self.total_len != total_len {
            bail!("fragment metadata changed within datagram");
        }
        let duplicate = self.fragments[frag_ix].is_some();
        self.fragments[frag_ix] = Some(chunk.to_vec());
        Ok(duplicate)
    }

    fn is_complete(&self) -> bool {
        self.fragments.iter().all(Option::is_some)
    }

    fn received_count(&self) -> usize {
        self.fragments.iter().filter(|fragment| fragment.is_some()).count()
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

    fn fragment_frame(datagram_id: u32, frag_ix: u16, frag_cnt: u16, total_len: u16, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(FRAG_HEADER_LEN + payload.len());
        frame.extend_from_slice(FRAG_MAGIC);
        frame.extend_from_slice(&datagram_id.to_le_bytes());
        frame.extend_from_slice(&frag_ix.to_le_bytes());
        frame.extend_from_slice(&frag_cnt.to_le_bytes());
        frame.extend_from_slice(&total_len.to_le_bytes());
        frame.extend_from_slice(payload);
        frame
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
    fn reliable_snapshot_uses_session_header_and_reassembles_byte_exact() {
        let datagram = kudp_datagram(329);
        let frames = fragment_datagram_with_options(&datagram, 7, false, true, false, 42).unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|frame| frame.starts_with(RELIABLE_FRAG_MAGIC)));
        assert!(frames.iter().all(|frame| frame.len() <= LORA_APP_MTU));

        let first = parse_fragment(&frames[0]).unwrap();
        assert_eq!(first.session_id, Some(42));
        assert_eq!(first.datagram_id, 7);

        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(&frames[1]).unwrap().is_none());
        let out = reassembler.push(&frames[0]).unwrap().unwrap();
        assert_eq!(out, datagram);
    }

    #[test]
    fn reliable_fragment_builds_matching_ack() {
        let datagram = kudp_datagram(329);
        let frames = fragment_datagram_with_options(&datagram, 77, false, true, false, 9).unwrap();
        let ack = ack_for_payload(&frames[0], 9).unwrap().unwrap();
        assert_eq!(parse_ack(&ack).unwrap(), Some(AckFrame { session_id: 9, datagram_id: 77, frag_ix: 0 }));
        assert!(ack_for_payload(&frames[0], 10).unwrap().is_none());
    }

    #[test]
    fn duplicate_reliable_fragment_is_counted_but_kept_safe() {
        let datagram = kudp_datagram(329);
        let frames = fragment_datagram_with_options(&datagram, 88, false, true, false, 4).unwrap();
        let mut stats = BridgeStats::default();
        let mut reassembler = Reassembler::default();

        assert!(reassembler.push_with_stats(&frames[0], &mut stats).unwrap().is_none());
        assert!(reassembler.push_with_stats(&frames[0], &mut stats).unwrap().is_none());
        assert_eq!(stats.duplicate_fragments, 1);
        let out = reassembler.push_with_stats(&frames[1], &mut stats).unwrap().unwrap();
        assert_eq!(out, datagram);
    }

    #[test]
    fn reliable_all_wraps_single_packet_delta_for_ack_recovery() {
        let datagram = kudp_datagram(200);
        let frames = fragment_datagram_with_options(&datagram, 91, false, true, true, 5).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].starts_with(RELIABLE_FRAG_MAGIC));
        let ack = ack_for_payload(&frames[0], 5).unwrap().unwrap();
        assert_eq!(parse_ack(&ack).unwrap(), Some(AckFrame { session_id: 5, datagram_id: 91, frag_ix: 0 }));

        let mut reassembler = Reassembler::default();
        let out = reassembler.push(&frames[0]).unwrap().unwrap();
        assert_eq!(out, datagram);
    }

    #[test]
    fn ack_parser_rejects_corrupted_ack() {
        let mut ack = make_ack(1, 2, 3);
        ack.push(0xff);
        assert!(parse_ack(&ack).is_err());
    }

    #[test]
    fn retry_budget_stops_after_configured_retries() {
        assert!(should_retry(0, 4));
        assert!(should_retry(3, 4));
        assert!(!should_retry(4, 4));
        assert!(!should_retry(0, 0));
    }

    #[test]
    fn group_id_overrides_session_id_for_reliable_domain() {
        let tx = TxArgs {
            serial: SerialArgs { serial: "/dev/null".into(), baud: DEFAULT_BAUD, read_timeout_ms: 250 },
            input: InputKind::File,
            file: None,
            udp_bind: None,
            fixed_prefix_hex: "000041000041".to_string(),
            inter_frame_delay_ms: None,
            reliable_fragments: true,
            reliable_all: false,
            retry_count: 4,
            ack_timeout_ms: 3_000,
            session_id: 1,
            group_id: Some(99),
            no_fragment: false,
            datagram_id: 1,
            count: 1,
        };
        let rx = RxArgs {
            serial: SerialArgs { serial: "/dev/null".into(), baud: DEFAULT_BAUD, read_timeout_ms: 250 },
            output: OutputKind::File,
            file: None,
            udp_target: None,
            count: 1,
            timeout_ms: 30_000,
            packet_idle_ms: 600,
            fixed_prefix_hex: "000041000041".to_string(),
            ack_fragments: true,
            session_id: 2,
            group_id: Some(100),
        };

        assert_eq!(tx.effective_session_id(), 99);
        assert_eq!(rx.effective_session_id(), 100);
    }

    #[test]
    fn out_of_order_fragments_reassemble() {
        let datagram = kudp_datagram(329);
        let frames = fragment_datagram(&datagram, 11, false).unwrap();

        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(&frames[1]).unwrap().is_none());
        let out = reassembler.push(&frames[0]).unwrap().unwrap();
        assert_eq!(out, datagram);
    }

    #[test]
    fn duplicate_fragment_does_not_complete_until_missing_fragment_arrives() {
        let datagram = kudp_datagram(329);
        let frames = fragment_datagram(&datagram, 12, false).unwrap();

        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(&frames[0]).unwrap().is_none());
        assert!(reassembler.push(&frames[0]).unwrap().is_none());
        assert_eq!(reassembler.pending_summary(), "datagram_id=12 received=1/2 total_len=329");

        let out = reassembler.push(&frames[1]).unwrap().unwrap();
        assert_eq!(out, datagram);
    }

    #[test]
    fn interleaved_datagram_ids_are_kept_separate() {
        let first = fragment_datagram(&kudp_datagram(329), 21, false).unwrap();
        let second = fragment_datagram(&kudp_datagram(330), 22, false).unwrap();

        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(&first[0]).unwrap().is_none());
        assert!(reassembler.push(&second[0]).unwrap().is_none());
        assert!(reassembler.pending_summary().contains("datagram_id=21 received=1/2 total_len=329"));
        assert!(reassembler.pending_summary().contains("datagram_id=22 received=1/2 total_len=330"));

        let first_out = reassembler.push(&first[1]).unwrap().unwrap();
        assert_eq!(first_out.len(), 329);
        assert!(reassembler.pending_summary().contains("datagram_id=22 received=1/2 total_len=330"));
    }

    #[test]
    fn malformed_fragments_are_rejected() {
        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(b"not-kudp-or-frag").is_err());
        assert!(reassembler.push(FRAG_MAGIC).is_err());
        assert!(reassembler.push(&fragment_frame(1, 0, 0, 16, b"x")).is_err());
        assert!(reassembler.push(&fragment_frame(1, 2, 2, 16, b"x")).is_err());
        assert!(reassembler.push(&fragment_frame(1, 0, 2, 0, b"x")).is_err());
        assert!(reassembler.push(&fragment_frame(1, 0, 2, 16, b"")).is_err());
    }

    #[test]
    fn fragment_metadata_changes_are_rejected() {
        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(&fragment_frame(33, 0, 2, 20, b"KUDPaaaa")).unwrap().is_none());
        let err = reassembler.push(&fragment_frame(33, 1, 2, 21, b"bbbb")).unwrap_err();
        assert!(err.to_string().contains("fragment metadata changed"));
    }

    #[test]
    fn reassembled_length_mismatch_is_reported() {
        let mut reassembler = Reassembler::default();
        assert!(reassembler.push(&fragment_frame(44, 0, 2, 20, b"KUDPaaaa")).unwrap().is_none());
        let err = reassembler.push(&fragment_frame(44, 1, 2, 20, b"bbbb")).unwrap_err();
        assert!(err.to_string().contains("reassembled length mismatch"));
    }

    #[test]
    fn pending_summary_exposes_missing_fragment_state() {
        let datagram = kudp_datagram(329);
        let frames = fragment_datagram(&datagram, 55, false).unwrap();
        let mut reassembler = Reassembler::default();

        assert!(!reassembler.has_pending());
        assert_eq!(reassembler.pending_summary(), "none");
        assert!(reassembler.push(&frames[0]).unwrap().is_none());
        assert!(reassembler.has_pending());
        assert_eq!(reassembler.pending_summary(), "datagram_id=55 received=1/2 total_len=329");
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
    fn rejects_too_short_waveshare_rx_packet() {
        assert!(strip_waveshare_rx(&[0x00, 0x00, 0x41]).is_err());
    }

    #[test]
    fn fixed_prefix_accepts_spaced_hex() {
        assert_eq!(parse_fixed_prefix("00 00 41 00 00 41").unwrap(), [0x00, 0x00, 0x41, 0x00, 0x00, 0x41]);
    }

    #[test]
    fn fixed_prefix_rejects_wrong_length_and_bad_hex() {
        assert!(parse_fixed_prefix("00 00 41").is_err());
        assert!(parse_fixed_prefix("00 00 41 00 00 zz").is_err());
    }
}
