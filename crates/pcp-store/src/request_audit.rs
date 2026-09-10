//! Task-local correlation at the transport boundary. No request content is retained.
use pcp_core::{AccessDecision, AccessSession, OperationTelemetry, RequestAuditMetadata};
use std::{
    collections::BTreeSet,
    future::Future,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Local completion record; SQLite supplies its own event ID and timestamp.
pub struct CompletedRequest {
    pub access: AccessSession,
    pub operation: String,
    pub scopes: Vec<String>,
    pub decision: AccessDecision,
    pub detail: Option<String>,
    pub telemetry: OperationTelemetry,
}

#[derive(Default)]
struct State {
    metadata: RequestAuditMetadata,
    scopes: BTreeSet<String>,
    denied: bool,
}
tokio::task_local! { static REQUEST: Arc<Mutex<State>>; static BACKGROUND: (); }

/// Mark a maintenance execution explicitly, including manual operator cycles.
pub async fn background<T>(work: impl Future<Output = T>) -> T {
    BACKGROUND.scope((), work).await
}

/// Attach identity before the audit writer's asynchronous queue. Spawned tasks
/// deliberately do not inherit this identity (background work is independent).
pub fn observe(
    scopes: &[String],
    operation: &str,
    decision: &AccessDecision,
    telemetry: Option<&OperationTelemetry>,
) -> Option<OperationTelemetry> {
    if telemetry.is_none()
        && REQUEST.try_with(|_| ()).is_err()
        && BACKGROUND.try_with(|_| ()).is_err()
    {
        return None;
    }
    let mut telemetry = telemetry.cloned();
    telemetry
        .get_or_insert_with(Default::default)
        .origin
        .get_or_insert_with(|| {
            if BACKGROUND.try_with(|_| ()).is_ok() {
                "background"
            } else {
                "local"
            }
            .to_owned()
        });
    let _ = REQUEST.try_with(|state| {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        state.scopes.extend(scopes.iter().cloned());
        state.metadata.internal_operations += 1;
        state.denied |= *decision == AccessDecision::Denied;
        if operation == "read_pages" {
            let count = telemetry.as_ref().and_then(|t| t.input_count).unwrap_or(0);
            state.metadata.page_visits += count;
            if count > 1 {
                state.metadata.batch_reads += 1;
            } else {
                state.metadata.single_reads += 1;
            }
        }
        telemetry.as_mut().unwrap().origin = Some("client_request".to_owned());
        telemetry.get_or_insert_with(Default::default).request = Some(RequestAuditMetadata {
            id: state.metadata.id.clone(),
            ..Default::default()
        });
    });
    telemetry
}

pub async fn capture<T>(
    access: &AccessSession,
    operation: String,
    work: impl Future<Output = anyhow::Result<T>>,
) -> (anyhow::Result<T>, CompletedRequest) {
    let state = Arc::new(Mutex::new(State::default()));
    state.lock().unwrap().metadata.id = uuid::Uuid::new_v4().to_string();
    let started = Instant::now();
    let outcome = REQUEST.scope(state.clone(), work).await;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.metadata.root = true;
    // Requests with no store calls still belong to the endpoint's explicit scopes.
    if state.scopes.is_empty() {
        state
            .scopes
            .extend(access.grants.iter().map(|grant| grant.namespace.clone()));
    }
    let event = CompletedRequest {
        access: access.clone(),
        operation,
        scopes: state.scopes.iter().cloned().collect(),
        decision: if outcome.is_ok() {
            AccessDecision::Allowed
        } else if state.denied {
            AccessDecision::Denied
        } else {
            AccessDecision::Failed
        },
        detail: outcome.as_ref().err().map(|_| "request failed".to_owned()),
        telemetry: OperationTelemetry {
            origin: Some("client_request".to_owned()),
            duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            request: Some(state.metadata.clone()),
            ..Default::default()
        },
    };
    (outcome, event)
}
