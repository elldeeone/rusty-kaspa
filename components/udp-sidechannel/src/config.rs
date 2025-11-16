use crate::frame::PayloadCaps;
use kaspa_consensus_core::network::{NetworkId, NetworkType};
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
    pub danger_accept_blocks: bool,
    pub block_mainnet_override: bool,
    pub discard_unsigned: bool,
    pub db_migrate: bool,
    pub retention_count: u32,
    pub retention_days: u32,
    pub max_digest_payload_bytes: u32,
    pub max_block_payload_bytes: u32,
    pub block_max_bytes: u32,
    pub log_verbosity: String,
    pub admin_remote_allowed: bool,
    pub admin_token_file: Option<PathBuf>,
    pub network_id: NetworkId,
}

impl UdpConfig {
    pub fn bind_target(&self) -> BindTarget {
        if let Some(path) = self.listen_unix.clone() {
            BindTarget::Unix(path)
        } else if let Some(addr) = self.listen {
            BindTarget::Udp(addr)
        } else {
            BindTarget::Disabled
        }
    }

    pub fn payload_caps(&self) -> PayloadCaps {
        PayloadCaps { digest: self.max_digest_payload_bytes, block: self.max_block_payload_bytes }
    }

    pub fn network_tag(&self) -> u8 {
        encode_network(&self.network_id)
    }

    pub fn initially_enabled(&self) -> bool {
        self.enable
    }

    pub fn blocks_allowed(&self) -> bool {
        if !(self.mode.allows_blocks() && self.danger_accept_blocks) {
            return false;
        }
        match self.network_id.network_type() {
            NetworkType::Mainnet => self.block_mainnet_override,
            _ => true,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BindTarget {
    Disabled,
    Udp(SocketAddr),
    Unix(PathBuf),
}

fn encode_network(id: &NetworkId) -> u8 {
    let base = match id.network_type() {
        NetworkType::Mainnet => 0x01,
        NetworkType::Testnet => 0x02,
        NetworkType::Devnet => 0x03,
        NetworkType::Simnet => 0x04,
    };
    let suffix = id.suffix().unwrap_or(0) as u8;
    base | (suffix << 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config(network: NetworkType) -> UdpConfig {
        UdpConfig {
            enable: true,
            listen: None,
            listen_unix: None,
            allow_non_local_bind: false,
            mode: UdpMode::Digest,
            max_kbps: 0,
            require_signature: false,
            allowed_signers: vec![],
            digest_queue: 0,
            block_queue: 0,
            danger_accept_blocks: false,
            block_mainnet_override: false,
            discard_unsigned: false,
            db_migrate: false,
            retention_count: 0,
            retention_days: 0,
            max_digest_payload_bytes: 0,
            max_block_payload_bytes: 0,
            block_max_bytes: 0,
            log_verbosity: String::new(),
            admin_remote_allowed: false,
            admin_token_file: None,
            network_id: NetworkId::new(network),
        }
    }

    #[test]
    fn blocks_disallowed_without_danger_flag_unit() {
        let mut cfg = base_config(NetworkType::Testnet);
        cfg.mode = UdpMode::Blocks;
        assert!(!cfg.blocks_allowed());
    }

    #[test]
    fn blocks_disallowed_on_mainnet_without_override_unit() {
        let mut cfg = base_config(NetworkType::Mainnet);
        cfg.mode = UdpMode::Both;
        cfg.danger_accept_blocks = true;
        assert!(!cfg.blocks_allowed());
    }

    #[test]
    fn blocks_allowed_on_testnet_with_danger_flag_unit() {
        let mut cfg = base_config(NetworkType::Testnet);
        cfg.mode = UdpMode::Both;
        cfg.danger_accept_blocks = true;
        assert!(cfg.blocks_allowed());
    }
}
