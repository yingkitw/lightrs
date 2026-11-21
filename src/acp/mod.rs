mod engine;
pub mod builtins;
pub mod v1;

#[cfg(feature = "acp-v2")]
pub mod v2;

use agent_client_protocol::Result;

use crate::cli::AcpProtocol;
use crate::config::AppConfig;

pub async fn serve(cfg: AppConfig, protocol: AcpProtocol) -> Result<()> {
    match protocol {
        AcpProtocol::V1 => v1::serve(cfg).await,
        #[cfg(feature = "acp-v2")]
        AcpProtocol::V2 => v2::serve(cfg).await,
    }
}