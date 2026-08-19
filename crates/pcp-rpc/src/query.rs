use anyhow::Result;
use async_trait::async_trait;
use pcp_client::PcpTenantApi;
pub use pcp_core::{QueryContextRequest, QueryContextResponse};

/// Retrieval services are Runtime-owned: the Store remains the authority for
/// Pages and access checks, while this service owns provider configuration,
/// bounded inference, and Context Pack assembly.
#[async_trait]
pub trait RuntimeQueryService: Send + Sync {
    async fn semantic_search(
        &self,
        client: &dyn PcpTenantApi,
        request: QueryContextRequest,
    ) -> Result<QueryContextResponse>;

    async fn match_intent(
        &self,
        client: &dyn PcpTenantApi,
        request: QueryContextRequest,
        effort: pcp_core::IntentEffort,
    ) -> Result<QueryContextResponse>;
}
