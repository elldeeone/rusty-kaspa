use kaspa_consensus_core::tx::{Transaction, TransactionId};
use kaspa_consensusmanager::ConsensusManager;
use kaspa_core::debug;
use kaspa_mining::mempool::tx::Orphan;
use kaspa_p2p_flows::flow_context::FlowContext;
use kaspa_udp_sidechannel::{TxSubmitError, TxSubmitFuture, UdpTxSubmitter};
use std::sync::Arc;

pub(crate) struct FlowTxSubmitter {
    flow_context: Arc<FlowContext>,
    consensus_manager: Arc<ConsensusManager>,
    unsafe_rpc: bool,
}

impl FlowTxSubmitter {
    pub(crate) fn new(flow_context: Arc<FlowContext>, consensus_manager: Arc<ConsensusManager>, unsafe_rpc: bool) -> Arc<Self> {
        Arc::new(Self { flow_context, consensus_manager, unsafe_rpc })
    }
}

impl UdpTxSubmitter for FlowTxSubmitter {
    fn submit(&self, transaction: Transaction, allow_orphan: bool) -> TxSubmitFuture {
        let flow_context = self.flow_context.clone();
        let consensus_manager = self.consensus_manager.clone();
        let unsafe_rpc = self.unsafe_rpc;
        Box::pin(async move {
            let transaction_id: TransactionId = transaction.id();
            let orphan = if unsafe_rpc && allow_orphan {
                Orphan::Allowed
            } else {
                if allow_orphan && !unsafe_rpc {
                    debug!("udp.event=tx_orphan_forbidden txid={}", transaction_id);
                }
                Orphan::Forbidden
            };
            let session = consensus_manager.consensus().unguarded_session();
            flow_context
                .submit_rpc_transaction(&session, transaction, orphan)
                .await
                .map_err(|err| TxSubmitError::new(err.to_string()))?;
            Ok(transaction_id)
        })
    }
}
