use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use workflow_serializer::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcUdpQueueSnapshot {
    pub capacity: u32,
    pub depth: u32,
}

impl Serializer for RpcUdpQueueSnapshot {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(u32, &self.capacity, writer)?;
        store!(u32, &self.depth, writer)?;
        Ok(())
    }
}

impl Deserializer for RpcUdpQueueSnapshot {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let capacity = load!(u32, reader)?;
        let depth = load!(u32, reader)?;
        Ok(Self { capacity, depth })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcUdpMetricEntry {
    pub label: String,
    pub value: u64,
}

impl Serializer for RpcUdpMetricEntry {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(String, &self.label, writer)?;
        store!(u64, &self.value, writer)?;
        Ok(())
    }
}

impl Deserializer for RpcUdpMetricEntry {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let label = load!(String, reader)?;
        let value = load!(u64, reader)?;
        Ok(Self { label, value })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcUdpDigestSummary {
    pub epoch: u64,
    pub pruning_point: String,
    pub pruning_proof_commitment: String,
    pub utxo_muhash: String,
    pub virtual_selected_parent: String,
    pub virtual_blue_score: u64,
    pub daa_score: u64,
    pub blue_work_hex: String,
    pub kept_headers_mmr_root: Option<String>,
    pub signer_id: u16,
    pub signature_valid: bool,
    pub recv_ts_ms: u64,
    pub source_id: u16,
}

impl Serializer for RpcUdpDigestSummary {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(u64, &self.epoch, writer)?;
        store!(String, &self.pruning_point, writer)?;
        store!(String, &self.pruning_proof_commitment, writer)?;
        store!(String, &self.utxo_muhash, writer)?;
        store!(String, &self.virtual_selected_parent, writer)?;
        store!(u64, &self.virtual_blue_score, writer)?;
        store!(u64, &self.daa_score, writer)?;
        store!(String, &self.blue_work_hex, writer)?;
        match &self.kept_headers_mmr_root {
            Some(value) => {
                store!(bool, &true, writer)?;
                store!(String, value, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        store!(u16, &self.signer_id, writer)?;
        store!(bool, &self.signature_valid, writer)?;
        store!(u64, &self.recv_ts_ms, writer)?;
        store!(u16, &self.source_id, writer)?;
        Ok(())
    }
}

impl Deserializer for RpcUdpDigestSummary {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let epoch = load!(u64, reader)?;
        let pruning_point = load!(String, reader)?;
        let pruning_proof_commitment = load!(String, reader)?;
        let utxo_muhash = load!(String, reader)?;
        let virtual_selected_parent = load!(String, reader)?;
        let virtual_blue_score = load!(u64, reader)?;
        let daa_score = load!(u64, reader)?;
        let blue_work_hex = load!(String, reader)?;
        let kept_headers_mmr_root = if load!(bool, reader)? { Some(load!(String, reader)?) } else { None };
        let signer_id = load!(u16, reader)?;
        let signature_valid = load!(bool, reader)?;
        let recv_ts_ms = load!(u64, reader)?;
        let source_id = load!(u16, reader)?;
        Ok(Self {
            epoch,
            pruning_point,
            pruning_proof_commitment,
            utxo_muhash,
            virtual_selected_parent,
            virtual_blue_score,
            daa_score,
            blue_work_hex,
            kept_headers_mmr_root,
            signer_id,
            signature_valid,
            recv_ts_ms,
            source_id,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcUdpSourceInfo {
    pub source_id: u16,
    pub last_epoch: u64,
    pub last_ts_ms: u64,
    pub signer_id: u16,
    pub signature_valid: bool,
}

impl Serializer for RpcUdpSourceInfo {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(u16, &self.source_id, writer)?;
        store!(u64, &self.last_epoch, writer)?;
        store!(u64, &self.last_ts_ms, writer)?;
        store!(u16, &self.signer_id, writer)?;
        store!(bool, &self.signature_valid, writer)?;
        Ok(())
    }
}

impl Deserializer for RpcUdpSourceInfo {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let source_id = load!(u16, reader)?;
        let last_epoch = load!(u64, reader)?;
        let last_ts_ms = load!(u64, reader)?;
        let signer_id = load!(u16, reader)?;
        let signature_valid = load!(bool, reader)?;
        Ok(Self { source_id, last_epoch, last_ts_ms, signer_id, signature_valid })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RpcUdpDivergenceInfo {
    pub detected: bool,
    pub last_mismatch_epoch: Option<u64>,
}

impl Serializer for RpcUdpDivergenceInfo {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(bool, &self.detected, writer)?;
        match self.last_mismatch_epoch {
            Some(epoch) => {
                store!(bool, &true, writer)?;
                store!(u64, &epoch, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        Ok(())
    }
}

impl Deserializer for RpcUdpDivergenceInfo {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let detected = load!(bool, reader)?;
        let last_mismatch_epoch = if load!(bool, reader)? { Some(load!(u64, reader)?) } else { None };
        Ok(Self { detected, last_mismatch_epoch })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetUdpIngestInfoRequest {
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Serializer for GetUdpIngestInfoRequest {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        match &self.auth_token {
            Some(token) => {
                store!(bool, &true, writer)?;
                store!(String, token, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        Ok(())
    }
}

impl Deserializer for GetUdpIngestInfoRequest {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let has_token = load!(bool, reader)?;
        let auth_token = if has_token { Some(load!(String, reader)?) } else { None };
        Ok(Self { auth_token })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUdpIngestInfoResponse {
    pub rpc_version: u16,
    pub enabled: bool,
    pub bind_address: Option<String>,
    pub bind_unix: Option<String>,
    pub allow_non_local: bool,
    pub mode: String,
    pub max_kbps: u32,
    pub digest_queue: RpcUdpQueueSnapshot,
    pub block_queue: RpcUdpQueueSnapshot,
    pub frames: Vec<RpcUdpMetricEntry>,
    pub drops: Vec<RpcUdpMetricEntry>,
    pub bytes_total: u64,
    pub rx_kbps: f64,
    pub last_frame_ts_ms: Option<u64>,
    pub frames_received: u64,
    pub last_digest: Option<RpcUdpDigestSummary>,
    pub divergence: RpcUdpDivergenceInfo,
    pub source_count: u32,
    pub sources: Vec<RpcUdpSourceInfo>,
    pub signature_failures: u64,
    pub skew_seconds: u64,
}

impl Serializer for GetUdpIngestInfoResponse {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(u16, &self.rpc_version, writer)?;
        store!(bool, &self.enabled, writer)?;
        match &self.bind_address {
            Some(addr) => {
                store!(bool, &true, writer)?;
                store!(String, addr, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        match &self.bind_unix {
            Some(path) => {
                store!(bool, &true, writer)?;
                store!(String, path, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        store!(bool, &self.allow_non_local, writer)?;
        store!(String, &self.mode, writer)?;
        store!(u32, &self.max_kbps, writer)?;
        workflow_serializer::serializer::Serializer::serialize(&self.digest_queue, writer)?;
        workflow_serializer::serializer::Serializer::serialize(&self.block_queue, writer)?;
        store!(u32, &(self.frames.len() as u32), writer)?;
        for entry in &self.frames {
            workflow_serializer::serializer::Serializer::serialize(entry, writer)?;
        }
        store!(u32, &(self.drops.len() as u32), writer)?;
        for entry in &self.drops {
            workflow_serializer::serializer::Serializer::serialize(entry, writer)?;
        }
        store!(u64, &self.bytes_total, writer)?;
        store!(f64, &self.rx_kbps, writer)?;
        match self.last_frame_ts_ms {
            Some(ts) => {
                store!(bool, &true, writer)?;
                store!(u64, &ts, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        store!(u64, &self.frames_received, writer)?;
        match &self.last_digest {
            Some(summary) => {
                store!(bool, &true, writer)?;
                workflow_serializer::serializer::Serializer::serialize(summary, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        workflow_serializer::serializer::Serializer::serialize(&self.divergence, writer)?;
        store!(u32, &self.source_count, writer)?;
        store!(u32, &(self.sources.len() as u32), writer)?;
        for entry in &self.sources {
            workflow_serializer::serializer::Serializer::serialize(entry, writer)?;
        }
        store!(u64, &self.signature_failures, writer)?;
        store!(u64, &self.skew_seconds, writer)?;
        Ok(())
    }
}

impl Deserializer for GetUdpIngestInfoResponse {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let rpc_version = load!(u16, reader)?;
        let enabled = load!(bool, reader)?;
        let bind_address = if load!(bool, reader)? { Some(load!(String, reader)?) } else { None };
        let bind_unix = if load!(bool, reader)? { Some(load!(String, reader)?) } else { None };
        let allow_non_local = load!(bool, reader)?;
        let mode = load!(String, reader)?;
        let max_kbps = load!(u32, reader)?;
        let digest_queue = <RpcUdpQueueSnapshot as workflow_serializer::serializer::Deserializer>::deserialize(reader)?;
        let block_queue = <RpcUdpQueueSnapshot as workflow_serializer::serializer::Deserializer>::deserialize(reader)?;
        let frames_len = load!(u32, reader)?;
        let mut frames = Vec::with_capacity(frames_len as usize);
        for _ in 0..frames_len {
            frames.push(<RpcUdpMetricEntry as workflow_serializer::serializer::Deserializer>::deserialize(reader)?);
        }
        let drops_len = load!(u32, reader)?;
        let mut drops = Vec::with_capacity(drops_len as usize);
        for _ in 0..drops_len {
            drops.push(<RpcUdpMetricEntry as workflow_serializer::serializer::Deserializer>::deserialize(reader)?);
        }
        let bytes_total = load!(u64, reader)?;
        let rx_kbps = load!(f64, reader)?;
        let last_frame_ts_ms = if load!(bool, reader)? { Some(load!(u64, reader)?) } else { None };
        let frames_received = load!(u64, reader)?;
        let last_digest = if load!(bool, reader)? {
            Some(<RpcUdpDigestSummary as workflow_serializer::serializer::Deserializer>::deserialize(reader)?)
        } else {
            None
        };
        let divergence = <RpcUdpDivergenceInfo as workflow_serializer::serializer::Deserializer>::deserialize(reader)?;
        let source_count = load!(u32, reader)?;
        let sources_len = load!(u32, reader)?;
        let mut sources = Vec::with_capacity(sources_len as usize);
        for _ in 0..sources_len {
            sources.push(<RpcUdpSourceInfo as workflow_serializer::serializer::Deserializer>::deserialize(reader)?);
        }
        let signature_failures = load!(u64, reader)?;
        let skew_seconds = load!(u64, reader)?;
        Ok(Self {
            rpc_version,
            enabled,
            bind_address,
            bind_unix,
            allow_non_local,
            mode,
            max_kbps,
            digest_queue,
            block_queue,
            frames,
            drops,
            bytes_total,
            rx_kbps,
            last_frame_ts_ms,
            frames_received,
            last_digest,
            divergence,
            source_count,
            sources,
            signature_failures,
            skew_seconds,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GetUdpDigestsRequest {
    pub from_epoch: Option<u64>,
    pub limit: Option<u32>,
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Serializer for GetUdpDigestsRequest {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        match self.from_epoch {
            Some(epoch) => {
                store!(bool, &true, writer)?;
                store!(u64, &epoch, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        match self.limit {
            Some(limit) => {
                store!(bool, &true, writer)?;
                store!(u32, &limit, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        match &self.auth_token {
            Some(token) => {
                store!(bool, &true, writer)?;
                store!(String, token, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        Ok(())
    }
}

impl Deserializer for GetUdpDigestsRequest {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let from_epoch = if load!(bool, reader)? { Some(load!(u64, reader)?) } else { None };
        let limit = if load!(bool, reader)? { Some(load!(u32, reader)?) } else { None };
        let has_token = load!(bool, reader)?;
        let auth_token = if has_token { Some(load!(String, reader)?) } else { None };
        Ok(Self { from_epoch, limit, auth_token })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcUdpDigestRecord {
    pub epoch: u64,
    pub kind: String,
    pub summary: RpcUdpDigestSummary,
    pub verified: bool,
}

impl Serializer for RpcUdpDigestRecord {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(u64, &self.epoch, writer)?;
        store!(String, &self.kind, writer)?;
        workflow_serializer::serializer::Serializer::serialize(&self.summary, writer)?;
        store!(bool, &self.verified, writer)?;
        Ok(())
    }
}

impl Deserializer for RpcUdpDigestRecord {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let epoch = load!(u64, reader)?;
        let kind = load!(String, reader)?;
        let summary = <RpcUdpDigestSummary as workflow_serializer::serializer::Deserializer>::deserialize(reader)?;
        let verified = load!(bool, reader)?;
        Ok(Self { epoch, kind, summary, verified })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetUdpDigestsResponse {
    pub digests: Vec<RpcUdpDigestRecord>,
}

impl Serializer for GetUdpDigestsResponse {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(u32, &(self.digests.len() as u32), writer)?;
        for digest in &self.digests {
            workflow_serializer::serializer::Serializer::serialize(digest, writer)?;
        }
        Ok(())
    }
}

impl Deserializer for GetUdpDigestsResponse {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let len = load!(u32, reader)?;
        let mut digests = Vec::with_capacity(len as usize);
        for _ in 0..len {
            digests.push(<RpcUdpDigestRecord as workflow_serializer::serializer::Deserializer>::deserialize(reader)?);
        }
        Ok(Self { digests })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UdpEnableRequest {
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Serializer for UdpEnableRequest {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        match &self.auth_token {
            Some(token) => {
                store!(bool, &true, writer)?;
                store!(String, token, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        Ok(())
    }
}

impl Deserializer for UdpEnableRequest {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let has_token = load!(bool, reader)?;
        let auth_token = if has_token { Some(load!(String, reader)?) } else { None };
        Ok(Self { auth_token })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UdpEnableResponse {
    pub previous_enabled: bool,
    pub enabled: bool,
    pub note: Option<String>,
}

impl Serializer for UdpEnableResponse {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(bool, &self.previous_enabled, writer)?;
        store!(bool, &self.enabled, writer)?;
        match &self.note {
            Some(note) => {
                store!(bool, &true, writer)?;
                store!(String, note, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        Ok(())
    }
}

impl Deserializer for UdpEnableResponse {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let previous_enabled = load!(bool, reader)?;
        let enabled = load!(bool, reader)?;
        let note = if load!(bool, reader)? { Some(load!(String, reader)?) } else { None };
        Ok(Self { previous_enabled, enabled, note })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UdpDisableRequest {
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl Serializer for UdpDisableRequest {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        match &self.auth_token {
            Some(token) => {
                store!(bool, &true, writer)?;
                store!(String, token, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        Ok(())
    }
}

impl Deserializer for UdpDisableRequest {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let has_token = load!(bool, reader)?;
        let auth_token = if has_token { Some(load!(String, reader)?) } else { None };
        Ok(Self { auth_token })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UdpDisableResponse {
    pub previous_enabled: bool,
    pub enabled: bool,
    pub note: Option<String>,
}

impl Serializer for UdpDisableResponse {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
        store!(bool, &self.previous_enabled, writer)?;
        store!(bool, &self.enabled, writer)?;
        match &self.note {
            Some(note) => {
                store!(bool, &true, writer)?;
                store!(String, note, writer)?;
            }
            None => store!(bool, &false, writer)?,
        }
        Ok(())
    }
}

impl Deserializer for UdpDisableResponse {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
        let previous_enabled = load!(bool, reader)?;
        let enabled = load!(bool, reader)?;
        let note = if load!(bool, reader)? { Some(load!(String, reader)?) } else { None };
        Ok(Self { previous_enabled, enabled, note })
    }
}
