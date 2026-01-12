use crate::protowire::{kaspad_request::Payload as RequestPayload, kaspad_response::Payload as ResponsePayload, *};
use kaspa_rpc_core::RpcError;
use workflow_core::enums::Describe;

macro_rules! payload_type_enum {
    ($(#[$meta:meta])* $vis:vis enum $name:ident {
    $($(#[$variant_meta:meta])* $variant_name:ident $(= $zero:literal)?,)*
    }) => {
        paste::paste! {
            $(#[$meta])*
            $vis enum $name {
                $($(#[$variant_meta])* $variant_name $(= $zero)?),*
            }

            impl $name {
                pub fn to_error_response(&self, error: RpcError) -> ResponsePayload {
                    match self {
                        $($name::$variant_name => [<$variant_name ResponseMessage>]::from(error).into()),*
                    }
                }
            }

            impl std::convert::From<&RequestPayload> for $name {
                fn from(value: &RequestPayload) -> Self {
                    match value {
                        RequestPayload::SubmitBlockRequest(_) => $name::SubmitBlock,
                        RequestPayload::GetBlockTemplateRequest(_) => $name::GetBlockTemplate,
                        RequestPayload::GetCurrentNetworkRequest(_) => $name::GetCurrentNetwork,
                        RequestPayload::GetBlockRequest(_) => $name::GetBlock,
                        RequestPayload::GetBlocksRequest(_) => $name::GetBlocks,
                        RequestPayload::GetInfoRequest(_) => $name::GetInfo,
                        RequestPayload::ShutdownRequest(_) => $name::Shutdown,
                        RequestPayload::GetPeerAddressesRequest(_) => $name::GetPeerAddresses,
                        RequestPayload::GetSinkRequest(_) => $name::GetSink,
                        RequestPayload::GetMempoolEntryRequest(_) => $name::GetMempoolEntry,
                        RequestPayload::GetMempoolEntriesRequest(_) => $name::GetMempoolEntries,
                        RequestPayload::GetConnectedPeerInfoRequest(_) => $name::GetConnectedPeerInfo,
                        RequestPayload::AddPeerRequest(_) => $name::AddPeer,
                        RequestPayload::SubmitTransactionRequest(_) => $name::SubmitTransaction,
                        RequestPayload::SubmitTransactionReplacementRequest(_) => $name::SubmitTransactionReplacement,
                        RequestPayload::GetSubnetworkRequest(_) => $name::GetSubnetwork,
                        RequestPayload::GetVirtualChainFromBlockRequest(_) => $name::GetVirtualChainFromBlock,
                        RequestPayload::GetBlockCountRequest(_) => $name::GetBlockCount,
                        RequestPayload::GetBlockDagInfoRequest(_) => $name::GetBlockDagInfo,
                        RequestPayload::ResolveFinalityConflictRequest(_) => $name::ResolveFinalityConflict,
                        RequestPayload::GetHeadersRequest(_) => $name::GetHeaders,
                        RequestPayload::GetUtxosByAddressesRequest(_) => $name::GetUtxosByAddresses,
                        RequestPayload::GetBalanceByAddressRequest(_) => $name::GetBalanceByAddress,
                        RequestPayload::GetBalancesByAddressesRequest(_) => $name::GetBalancesByAddresses,
                        RequestPayload::GetSinkBlueScoreRequest(_) => $name::GetSinkBlueScore,
                        RequestPayload::BanRequest(_) => $name::Ban,
                        RequestPayload::UnbanRequest(_) => $name::Unban,
                        RequestPayload::EstimateNetworkHashesPerSecondRequest(_) => $name::EstimateNetworkHashesPerSecond,
                        RequestPayload::GetMempoolEntriesByAddressesRequest(_) => $name::GetMempoolEntriesByAddresses,
                        RequestPayload::GetCoinSupplyRequest(_) => $name::GetCoinSupply,
                        RequestPayload::PingRequest(_) => $name::Ping,
                        RequestPayload::GetMetricsRequest(_) => $name::GetMetrics,
                        RequestPayload::GetConnectionsRequest(_) => $name::GetConnections,
                        RequestPayload::GetSystemInfoRequest(_) => $name::GetSystemInfo,
                        RequestPayload::GetServerInfoRequest(_) => $name::GetServerInfo,
                        RequestPayload::GetSyncStatusRequest(_) => $name::GetSyncStatus,
                        RequestPayload::GetDaaScoreTimestampEstimateRequest(_) => $name::GetDaaScoreTimestampEstimate,
                        RequestPayload::GetFeeEstimateRequest(_) => $name::GetFeeEstimate,
                        RequestPayload::GetFeeEstimateExperimentalRequest(_) => $name::GetFeeEstimateExperimental,
                        RequestPayload::GetCurrentBlockColorRequest(_) => $name::GetCurrentBlockColor,
                        RequestPayload::GetUtxoReturnAddressRequest(_) => $name::GetUtxoReturnAddress,
                        RequestPayload::GetVirtualChainFromBlockV2Request(_) => $name::GetVirtualChainFromBlockV2,
                        RequestPayload::GetUdpIngestInfoRequest(_) => $name::GetUdpIngestInfo,
                        RequestPayload::GetUdpDigestsRequest(_) => $name::GetUdpDigests,
                        RequestPayload::UdpEnableRequest(_) => $name::UdpEnable,
                        RequestPayload::UdpDisableRequest(_) => $name::UdpDisable,
                        RequestPayload::UdpUpdateSignersRequest(_) => $name::UdpUpdateSigners,
                        RequestPayload::NotifyBlockAddedRequest(_) => $name::NotifyBlockAdded,
                        RequestPayload::NotifyNewBlockTemplateRequest(_) => $name::NotifyNewBlockTemplate,
                        RequestPayload::NotifyFinalityConflictRequest(_) => $name::NotifyFinalityConflict,
                        RequestPayload::NotifyUtxosChangedRequest(_) => $name::NotifyUtxosChanged,
                        RequestPayload::NotifySinkBlueScoreChangedRequest(_) => $name::NotifySinkBlueScoreChanged,
                        RequestPayload::NotifyPruningPointUtxoSetOverrideRequest(_) => $name::NotifyPruningPointUtxoSetOverride,
                        RequestPayload::NotifyVirtualDaaScoreChangedRequest(_) => $name::NotifyVirtualDaaScoreChanged,
                        RequestPayload::NotifyVirtualChainChangedRequest(_) => $name::NotifyVirtualChainChanged,
                        RequestPayload::StopNotifyingUtxosChangedRequest(_) => $name::StopNotifyingUtxosChanged,
                        RequestPayload::StopNotifyingPruningPointUtxoSetOverrideRequest(_) => $name::StopNotifyingPruningPointUtxoSetOverride,
                    }
                }
            }

            impl TryFrom<&ResponsePayload> for $name {
                type Error = ();

                fn try_from(value: &ResponsePayload) -> Result<Self, Self::Error> {
                    match value {
                        $(ResponsePayload::[<$variant_name Response>](_) => Ok($name::$variant_name)),*,
                        _ => Err(())
                    }
                }
            }

        }
    }
}

payload_type_enum! {
#[repr(u8)]
#[derive(Describe, Debug, Copy, Clone, Eq, Hash, PartialEq)]
pub enum KaspadPayloadOps {
    SubmitBlock = 0,
    GetBlockTemplate,
    GetCurrentNetwork,
    GetBlock,
    GetBlocks,
    GetInfo,
    Shutdown,
    GetPeerAddresses,
    GetSink,
    GetMempoolEntry,
    GetMempoolEntries,
    GetConnectedPeerInfo,
    AddPeer,
    SubmitTransaction,
    SubmitTransactionReplacement,
    GetSubnetwork,
    GetVirtualChainFromBlock,
    GetBlockCount,
    GetBlockDagInfo,
    ResolveFinalityConflict,
    GetHeaders,
    GetUtxosByAddresses,
    GetBalanceByAddress,
    GetBalancesByAddresses,
    GetSinkBlueScore,
    Ban,
    Unban,
    EstimateNetworkHashesPerSecond,
    GetMempoolEntriesByAddresses,
    GetCoinSupply,
    Ping,
    GetMetrics,
    GetConnections,
    GetSystemInfo,
    GetServerInfo,
    GetSyncStatus,
    GetDaaScoreTimestampEstimate,
    GetFeeEstimate,
    GetFeeEstimateExperimental,
    GetCurrentBlockColor,
    GetUtxoReturnAddress,
    GetVirtualChainFromBlockV2,
    GetUdpIngestInfo,
    GetUdpDigests,
    UdpEnable,
    UdpDisable,
    UdpUpdateSigners,

    // Subscription commands for starting/stopping notifications
    NotifyBlockAdded,
    NotifyNewBlockTemplate,
    NotifyFinalityConflict,
    NotifyUtxosChanged,
    NotifySinkBlueScoreChanged,
    NotifyPruningPointUtxoSetOverride,
    NotifyVirtualDaaScoreChanged,
    NotifyVirtualChainChanged,

    // Legacy stop subscription commands
    StopNotifyingUtxosChanged,
    StopNotifyingPruningPointUtxoSetOverride,

    // Please note:
    // Notification payloads existing in ResponsePayload are not considered valid ops.
    // The conversion from a notification ResponsePayload into KaspadPayloadOps fails.
}
}
