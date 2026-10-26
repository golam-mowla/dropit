use crate::config::DropItConfig;
use crate::crypto::TlsIdentity;

pub struct ServerState {
    pub config: DropItConfig,
    pub identity: TlsIdentity,
}

impl ServerState {
    pub fn new(config: DropItConfig, identity: TlsIdentity) -> Self {
        Self { config, identity }
    }
}
