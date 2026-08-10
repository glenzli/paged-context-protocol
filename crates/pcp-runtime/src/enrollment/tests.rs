use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use pcp_client::PcpApi;
use pcp_core::{AccessPermission, AccessPrincipalType};
use pcp_rpc::{
    BeginEnrollmentParams, EnrollmentAdminClient, EnrollmentAdminResult, EnrollmentClient,
    EnrollmentClientClaim, EnrollmentPrincipalClaim, EnrollmentResult, EnrollmentStatusParams,
    OpenEnrollmentSessionParams, PCP_ENROLLMENT_PROTOCOL_ID, RemotePcpClient, RequestedAccess,
    RequestedAccessMode,
};
use pcp_sqlite::SqlitePcpStore;
use pcp_store::PcpStore;

use crate::{EnrollmentConfig, ObserverConfig, ObserverService};

#[tokio::test]
async fn enrollment_approves_identity_bound_session_and_survives_generation_change() {
    let root = test_root("lifecycle");
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open enrollment test store"),
    );
    let owner_id = store.owner_id().to_owned();
    let store: Arc<dyn PcpStore> = store;
    let mut observer_config = ObserverConfig::for_test(root.clone(), owner_id.clone());
    observer_config.enrollment_enabled = true;
    let enrollment_config = EnrollmentConfig::for_test(root.clone());
    let admin_socket = enrollment_config.admin_socket_path.clone();
    let state_path = enrollment_config.state_path.clone();
    let mut observer = ObserverService::start(
        observer_config.clone(),
        enrollment_config,
        Arc::clone(&store),
    )
    .await
    .expect("start enrollment provider")
    .expect("provider enabled");
    let first_generation = observer.generation().to_owned();
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(observer_config.manifest_path()).expect("read discovery manifest"),
    )
    .expect("decode discovery manifest");
    assert!(manifest["offers"].as_array().unwrap().iter().any(|offer| {
        offer["protocol"] == PCP_ENROLLMENT_PROTOCOL_ID
            && offer["endpoint"]
                .as_str()
                .is_some_and(|value| value.starts_with("sockets/"))
    }));

    let credential = "ab".repeat(32);
    let public = EnrollmentClient::new(observer.socket_path());
    let pending = public
        .begin(begin_params(&credential))
        .await
        .expect("begin enrollment");
    let request_id = match pending.result {
        EnrollmentResult::Pending { request_id, .. } => request_id,
        other => panic!("expected pending enrollment, got {other:?}"),
    };
    let state_text = fs::read_to_string(&state_path).expect("read enrollment state");
    assert!(!state_text.contains(&credential));
    assert_eq!(
        fs::metadata(&state_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let admin = EnrollmentAdminClient::new(&admin_socket);
    let snapshot = admin.snapshot().await.expect("read pending enrollments");
    let snapshot_text = serde_json::to_string(&snapshot).expect("encode admin snapshot");
    assert!(!snapshot_text.contains(&credential));
    assert!(!snapshot_text.contains("credential_hash"));
    match snapshot.result {
        EnrollmentAdminResult::Snapshot {
            pending,
            registrations,
        } => {
            assert_eq!(pending.len(), 1);
            assert!(registrations.is_empty());
            assert_eq!(pending[0].request_id, request_id);
        }
        other => panic!("expected admin snapshot, got {other:?}"),
    }
    admin
        .approve(request_id.clone())
        .await
        .expect("approve enrollment");
    let active = public
        .status(EnrollmentStatusParams {
            request_id: request_id.clone(),
            credential: credential.clone(),
        })
        .await
        .expect("open approved enrollment");
    let first_session = match active.result {
        EnrollmentResult::Active { session } => session,
        other => panic!("expected active enrollment, got {other:?}"),
    };
    assert_eq!(first_session.service.generation, first_generation);
    assert!(std::path::Path::new(&first_session.endpoint).is_relative());
    assert!(
        first_session
            .access
            .allows(&format!("user:{owner_id}"), AccessPermission::ReadDetail)
    );
    let remote =
        RemotePcpClient::connect_expected(root.join(&first_session.endpoint), "host:symbiont-d")
            .await
            .expect("connect identity-bound session");
    assert_eq!(remote.access(), &first_session.access);
    let idempotent = public
        .begin(begin_params(&credential))
        .await
        .expect("repeat approved begin");
    match idempotent.result {
        EnrollmentResult::Active { session } => {
            assert_eq!(session.registration_id, first_session.registration_id);
            assert_eq!(session.endpoint, first_session.endpoint);
        }
        other => panic!("expected active idempotent begin, got {other:?}"),
    }

    let registration_id = first_session.registration_id.clone();
    observer.shutdown().await.expect("stop first provider");
    let enrollment_config = EnrollmentConfig::for_test(root.clone());
    let mut replacement =
        ObserverService::start(observer_config, enrollment_config, Arc::clone(&store))
            .await
            .expect("restart enrollment provider")
            .expect("replacement enabled");
    let replacement_public = EnrollmentClient::new(replacement.socket_path());
    let reopened = replacement_public
        .open_session(OpenEnrollmentSessionParams {
            registration_id: registration_id.clone(),
            credential: credential.clone(),
        })
        .await
        .expect("reopen persisted registration");
    let replacement_session = match reopened.result {
        EnrollmentResult::Active { session } => session,
        other => panic!("expected reopened enrollment, got {other:?}"),
    };
    assert_ne!(replacement_session.service.generation, first_generation);
    assert_eq!(replacement_session.registration_id, registration_id);
    let replacement_remote = RemotePcpClient::connect_expected(
        root.join(&replacement_session.endpoint),
        "host:symbiont-d",
    )
    .await
    .expect("connect replacement identity-bound session");

    let replacement_admin = EnrollmentAdminClient::new(&admin_socket);
    replacement_admin
        .revoke(registration_id.clone())
        .await
        .expect("revoke enrollment");
    assert!(replacement_remote.page_count(Vec::new()).await.is_err());
    assert!(
        replacement_public
            .open_session(OpenEnrollmentSessionParams {
                registration_id,
                credential,
            })
            .await
            .is_err()
    );
    replacement.shutdown().await.expect("stop replacement");
    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn enrollment_requires_the_client_credential_for_status() {
    let root = test_root("credential");
    let store = Arc::new(
        SqlitePcpStore::open(root.join("context.sqlite3"))
            .await
            .expect("open credential test store"),
    );
    let owner_id = store.owner_id().to_owned();
    let store: Arc<dyn PcpStore> = store;
    let mut observer_config = ObserverConfig::for_test(root.clone(), owner_id);
    observer_config.enrollment_enabled = true;
    let mut observer = ObserverService::start(
        observer_config,
        EnrollmentConfig::for_test(root.clone()),
        store,
    )
    .await
    .expect("start enrollment provider")
    .expect("provider enabled");
    let credential = "cd".repeat(32);
    let public = EnrollmentClient::new(observer.socket_path());
    let request_id = match public
        .begin(begin_params(&credential))
        .await
        .expect("begin enrollment")
        .result
    {
        EnrollmentResult::Pending { request_id, .. } => request_id,
        other => panic!("expected pending enrollment, got {other:?}"),
    };
    assert!(
        public
            .status(EnrollmentStatusParams {
                request_id,
                credential: "ef".repeat(32),
            })
            .await
            .is_err()
    );
    let mut duplicate_scope = begin_params(&"12".repeat(32));
    duplicate_scope.requested_access.scopes = vec!["user:self".to_owned(); 2];
    assert!(public.begin(duplicate_scope).await.is_err());
    observer.shutdown().await.expect("stop provider");
    let _ = tokio::fs::remove_dir_all(root).await;
}

fn begin_params(credential: &str) -> BeginEnrollmentParams {
    BeginEnrollmentParams {
        client: EnrollmentClientClaim {
            principal: EnrollmentPrincipalClaim {
                principal_id: "host:symbiont-d".to_owned(),
                principal_type: AccessPrincipalType::Host,
                display_name: Some("Symbiont".to_owned()),
            },
        },
        requested_access: RequestedAccess {
            mode: RequestedAccessMode::Admin,
            scopes: vec![
                "user:self".to_owned(),
                "project:symbiont-d".to_owned(),
                "conversation:symbiont-d".to_owned(),
            ],
            allow_cross_scope_derivation: true,
        },
        credential: credential.to_owned(),
    }
}

fn test_root(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::path::PathBuf::from("/tmp").join(format!("pcpe-{label}-{}", nonce % 1_000_000_000));
    fs::create_dir_all(&root).expect("create enrollment test root");
    root
}
