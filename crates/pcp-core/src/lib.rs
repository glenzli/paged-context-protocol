mod access;
mod model;
mod request;
mod retention;

pub use access::{
    AccessAuditEvent, AccessDecision, AccessPermission, AccessPrincipal, AccessPrincipalType,
    AccessSession, OperationTelemetry, ScopeGrant,
};
pub use model::{
    Actor, ActorType, BrowseIndexOrder, Capabilities, GraphEdgeDirection, GraphEdgeKind,
    GraphSearchEdge, LifecycleStatus, PACKED_PAGE_MEDIA_TYPE, Page, PageMutability, PagePayload,
    PageRevision, PageSummary, PageValidity, PageValidityHint, Projection, ProvenanceEvent,
    ReadPage, Relation, Revision, Scope, SearchHit, SearchMode, SearchResult, SearchTermMatch,
    SourceRef, SourceSpan, ValidityStanding, WriteResult, WriteSummaryResult, WriteValidityResult,
};
pub use request::{
    AssessPageValidityRequest, CreateScopeRequest, IngestPageRequest, InitialRelation,
    LinkPagesRequest, PackPagesRequest, PageRevisionRef, ReadPagesRequest, RevisePageRequest,
    SearchFilters, SearchPagesRequest, WritePageRequest, WriteSummaryRequest,
    default_search_projections,
};
pub use retention::{
    CollectRevisionRetentionRequest, PlanRevisionRetentionRequest, ProtectedRevisionSample,
    PutRevisionRetentionLeaseRequest, RetentionPolicy, RetentionProtectionReason,
    RetentionReasonCount, RevisionCollectionResult, RevisionRetentionCandidate,
    RevisionRetentionLease, RevisionRetentionPlan,
};

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{CreateScopeRequest, SourceRef};

    #[test]
    fn v08_source_and_scope_wire_omit_redundant_identity_and_visibility() {
        let source = serde_json::to_value(SourceRef {
            provider_id: "tenant:photos".to_owned(),
            locator: "opaque-photo-42".to_owned(),
            media_type: Some("image/jpeg".to_owned()),
            content_digest: None,
        })
        .expect("serialize SourceRef");
        assert_eq!(
            source,
            json!({
                "providerId": "tenant:photos",
                "locator": "opaque-photo-42",
                "mediaType": "image/jpeg"
            })
        );

        let scope = serde_json::to_value(CreateScopeRequest {
            namespace: "project:example".to_owned(),
            display_name: "Example".to_owned(),
            description: None,
            parent_namespace: None,
        })
        .expect("serialize Scope request");
        assert!(scope.get("identityId").is_none());
        assert!(scope.get("visibility").is_none());
    }
}
