use crate::{flow_context::FlowContext, flow_trait::Flow};
use kaspa_consensus_core::block::Block;
use kaspa_core::{debug, warn};
use kaspa_p2p_lib::{
    common::ProtocolError, convert::error::ConversionError, dequeue, pb::kaspad_message::Payload, IncomingRoute,
    KaspadMessagePayloadType, Router,
};
use std::sync::Arc;

pub struct BlockInjectionFlow {
    ctx: FlowContext,
    router: Arc<Router>,
    incoming_route: IncomingRoute,
}

impl BlockInjectionFlow {
    pub fn new(ctx: FlowContext, router: Arc<Router>) -> Self {
        let incoming_route = router.subscribe(vec![KaspadMessagePayloadType::Block]);
        Self { ctx, router, incoming_route }
    }
}

#[async_trait::async_trait]
impl Flow for BlockInjectionFlow {
    fn router(&self) -> Option<Arc<Router>> {
        Some(self.router.clone())
    }

    async fn start(&mut self) -> Result<(), ProtocolError> {
        loop {
            let msg = dequeue!(self.incoming_route, Payload::Block)?;
            let block: Block = msg.try_into().map_err(|err: ConversionError| ProtocolError::OtherOwned(err.to_string()))?;
            let session = self.ctx.consensus().session().await;
            if let Err(err) = self.ctx.submit_rpc_block(&session, block.clone()).await {
                println!("satellite block {} rejected: {}", block.hash(), err);
                warn!("satellite block {} rejected: {}", block.hash(), err);
                return Err(err);
            }
            debug!("satellite block {} accepted", block.hash());
        }
    }
}
