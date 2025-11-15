//!
//! Generic connection trait representing a connection to a client (where available).
//!

use std::{net::SocketAddr, sync::Arc};

pub trait RpcConnection: Send + Sync {
    fn id(&self) -> u64;
    fn peer_address(&self) -> Option<SocketAddr> {
        None
    }
}

pub type DynRpcConnection = Arc<dyn RpcConnection>;
