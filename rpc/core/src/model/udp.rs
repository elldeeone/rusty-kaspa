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
}

impl Serializer for GetUdpIngestInfoResponse {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        store!(u16, &0, writer)?;
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
        Ok(())
    }
}

impl Deserializer for GetUdpIngestInfoResponse {
    fn deserialize<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let _version = load!(u16, reader)?;
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
        Ok(Self {
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
        })
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

cfg_if::cfg_if! {
    if #[cfg(feature = "wasm32-sdk")] {
        use wasm_bindgen::prelude::*;

        #[wasm_bindgen(typescript_custom_section)]
        const TS_RPC_UDP_QUEUE_SNAPSHOT: &'static str = r#"
            /**
             * UDP queue snapshot.
             *
             * @category Node RPC
             */
            export interface IRpcUdpQueueSnapshot {
                capacity: number;
                depth: number;
            }
        "#;

        #[wasm_bindgen(typescript_custom_section)]
        const TS_RPC_UDP_METRIC_ENTRY: &'static str = r#"
            /**
             * UDP metric entry.
             *
             * @category Node RPC
             */
            export interface IRpcUdpMetricEntry {
                label: string;
                value: bigint;
            }
        "#;
    }
}
