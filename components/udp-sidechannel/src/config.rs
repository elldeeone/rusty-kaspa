use kaspa_consensus_core::network::NetworkId;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdpMode {
    Digest,
    Blocks,
    Both,
}

impl UdpMode {
    pub fn allows_blocks(&self) -> bool {
        matches!(self, Self::Blocks | Self::Both)
    }

    pub fn allows_digest(&self) -> bool {
        matches!(self, Self::Digest | Self::Both)
    }
}

#[derive(Debug, Clone)]
pub struct UdpConfig {
    pub enable: bool,
    pub listen: Option<SocketAddr>,
    pub listen_unix: Option<PathBuf>,
    pub allow_non_local_bind: bool,
    pub mode: UdpMode,
    pub max_kbps: u32,
    pub require_signature: bool,
    pub allowed_signers: Vec<String>,
    pub digest_queue: usize,
    pub block_queue: usize,
    pub discard_unsigned: bool,
    pub db_migrate: bool,
    pub retention_count: u32,
    pub retention_days: u32,
    pub log_verbosity: String,
    pub admin_remote_allowed: bool,
    pub admin_token_file: Option<PathBuf>,
    pub network_id: NetworkId,
}

impl UdpConfig {
    pub fn bind_target(&self) -> BindTarget {
        if !self.enable {
            return BindTarget::Disabled;
        }

        if let Some(path) = self.listen_unix.clone() {
            BindTarget::Unix(path)
        } else if let Some(addr) = self.listen {
            BindTarget::Udp(addr)
        } else {
            BindTarget::Disabled
        }
    }
}

#[derive(Debug, Clone)]
pub enum BindTarget {
    Disabled,
    Udp(SocketAddr),
    Unix(PathBuf),
}
