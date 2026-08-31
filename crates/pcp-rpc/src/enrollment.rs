use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use pcp_core::{AccessPrincipalType, AccessSession};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    time::timeout,
};

pub const PCP_ENROLLMENT_PROTOCOL_ID: &str = "pcp.runtime.enrollment";
pub const PCP_ENROLLMENT_PROTOCOL_VERSION: &str = "20260810.1";
pub const ENROLLMENT_REQUEST_SCHEMA: &str = "pcp.runtime.enrollment.request";
pub const ENROLLMENT_RESPONSE_SCHEMA: &str = "pcp.runtime.enrollment.response";
pub const ENROLLMENT_ERROR_SCHEMA: &str = "pcp.runtime.enrollment.error";
pub const ENROLLMENT_ADMIN_REQUEST_SCHEMA: &str = "pcp.runtime.enrollment.admin.request";
pub const ENROLLMENT_ADMIN_RESPONSE_SCHEMA: &str = "pcp.runtime.enrollment.admin.response";
pub const ENROLLMENT_ADMIN_ERROR_SCHEMA: &str = "pcp.runtime.enrollment.admin.error";
pub const ENROLLMENT_MAX_REQUEST_FRAME_BYTES: usize = 16 * 1024;
pub const ENROLLMENT_MAX_RESPONSE_FRAME_BYTES: usize = 128 * 1024;
const ENROLLMENT_CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedAccessMode {
    Observe,
    Read,
    Audit,
    Contribute,
    Write,
    Repair,
    Admin,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedAccess {
    pub mode: RequestedAccessMode,
    pub scopes: Vec<String>,
    #[serde(default)]
    pub allow_cross_scope_derivation: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentClientClaim {
    pub principal: EnrollmentPrincipalClaim,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentPrincipalClaim {
    pub principal_id: String,
    pub principal_type: AccessPrincipalType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BeginEnrollmentParams {
    pub client: EnrollmentClientClaim,
    pub requested_access: RequestedAccess,
    pub credential: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentStatusParams {
    pub request_id: String,
    pub credential: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenEnrollmentSessionParams {
    pub registration_id: String,
    pub credential: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRequest {
    pub schema: String,
    pub schema_version: String,
    pub operation: String,
    pub params: Value,
}

impl EnrollmentRequest {
    pub fn begin(params: BeginEnrollmentParams) -> Result<Self> {
        Self::new("begin", params)
    }

    pub fn status(params: EnrollmentStatusParams) -> Result<Self> {
        Self::new("status", params)
    }

    pub fn open_session(params: OpenEnrollmentSessionParams) -> Result<Self> {
        Self::new("open_session", params)
    }

    fn new(operation: &str, params: impl Serialize) -> Result<Self> {
        Ok(Self {
            schema: ENROLLMENT_REQUEST_SCHEMA.to_owned(),
            schema_version: PCP_ENROLLMENT_PROTOCOL_VERSION.to_owned(),
            operation: operation.to_owned(),
            params: serde_json::to_value(params).context("encode PCP enrollment request params")?,
        })
    }

    pub fn decode_params<T: DeserializeOwned>(&self, operation: &str) -> Result<T> {
        validate_request_header(
            &self.schema,
            &self.schema_version,
            &self.operation,
            ENROLLMENT_REQUEST_SCHEMA,
            operation,
        )?;
        serde_json::from_value(self.params.clone()).context("decode PCP enrollment request params")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentServiceIdentity {
    pub kind: String,
    pub instance_id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentSession {
    pub registration_id: String,
    pub service: EnrollmentServiceIdentity,
    pub binding: String,
    pub endpoint: String,
    pub access: AccessSession,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EnrollmentResult {
    Pending {
        request_id: String,
        requested_at: String,
        expires_at: String,
    },
    Rejected {
        request_id: String,
    },
    Active {
        session: EnrollmentSession,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnrollmentResponse {
    pub schema: String,
    pub schema_version: String,
    pub operation: String,
    pub result: EnrollmentResult,
}

impl EnrollmentResponse {
    pub fn new(operation: impl Into<String>, result: EnrollmentResult) -> Self {
        Self {
            schema: ENROLLMENT_RESPONSE_SCHEMA.to_owned(),
            schema_version: PCP_ENROLLMENT_PROTOCOL_VERSION.to_owned(),
            operation: operation.into(),
            result,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingEnrollmentView {
    pub request_id: String,
    pub client: EnrollmentClientClaim,
    pub requested_access: RequestedAccess,
    pub requested_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RegisteredClientView {
    pub registration_id: String,
    pub client: EnrollmentClientClaim,
    pub approved_access: RequestedAccess,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentAdminRequest {
    pub schema: String,
    pub schema_version: String,
    pub operation: String,
    pub params: Value,
}

impl EnrollmentAdminRequest {
    pub fn snapshot() -> Result<Self> {
        Self::new("snapshot", EmptyParams {})
    }

    pub fn approve(request_id: impl Into<String>) -> Result<Self> {
        Self::new(
            "approve",
            EnrollmentRequestIdParams {
                request_id: request_id.into(),
            },
        )
    }

    pub fn reject(request_id: impl Into<String>) -> Result<Self> {
        Self::new(
            "reject",
            EnrollmentRequestIdParams {
                request_id: request_id.into(),
            },
        )
    }

    pub fn revoke(registration_id: impl Into<String>) -> Result<Self> {
        Self::new(
            "revoke",
            EnrollmentRegistrationIdParams {
                registration_id: registration_id.into(),
            },
        )
    }

    fn new(operation: &str, params: impl Serialize) -> Result<Self> {
        Ok(Self {
            schema: ENROLLMENT_ADMIN_REQUEST_SCHEMA.to_owned(),
            schema_version: PCP_ENROLLMENT_PROTOCOL_VERSION.to_owned(),
            operation: operation.to_owned(),
            params: serde_json::to_value(params)
                .context("encode PCP enrollment admin request params")?,
        })
    }

    pub fn decode_params<T: DeserializeOwned>(&self, operation: &str) -> Result<T> {
        validate_request_header(
            &self.schema,
            &self.schema_version,
            &self.operation,
            ENROLLMENT_ADMIN_REQUEST_SCHEMA,
            operation,
        )?;
        serde_json::from_value(self.params.clone())
            .context("decode PCP enrollment admin request params")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyParams {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRequestIdParams {
    pub request_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRegistrationIdParams {
    pub registration_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EnrollmentAdminResult {
    Snapshot {
        pending: Vec<PendingEnrollmentView>,
        registrations: Vec<RegisteredClientView>,
    },
    Applied,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnrollmentAdminResponse {
    pub schema: String,
    pub schema_version: String,
    pub operation: String,
    pub result: EnrollmentAdminResult,
}

impl EnrollmentAdminResponse {
    pub fn new(operation: impl Into<String>, result: EnrollmentAdminResult) -> Self {
        Self {
            schema: ENROLLMENT_ADMIN_RESPONSE_SCHEMA.to_owned(),
            schema_version: PCP_ENROLLMENT_PROTOCOL_VERSION.to_owned(),
            operation: operation.into(),
            result,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnrollmentError {
    pub schema: String,
    pub schema_version: String,
    pub code: String,
    pub message: String,
}

impl EnrollmentError {
    pub fn public(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ENROLLMENT_ERROR_SCHEMA, code, message)
    }

    pub fn admin(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ENROLLMENT_ADMIN_ERROR_SCHEMA, code, message)
    }

    fn new(schema: &str, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            schema: schema.to_owned(),
            schema_version: PCP_ENROLLMENT_PROTOCOL_VERSION.to_owned(),
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EnrollmentClient {
    socket_path: PathBuf,
}

impl EnrollmentClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub async fn begin(&self, params: BeginEnrollmentParams) -> Result<EnrollmentResponse> {
        self.exchange(&EnrollmentRequest::begin(params)?).await
    }

    pub async fn status(&self, params: EnrollmentStatusParams) -> Result<EnrollmentResponse> {
        self.exchange(&EnrollmentRequest::status(params)?).await
    }

    pub async fn open_session(
        &self,
        params: OpenEnrollmentSessionParams,
    ) -> Result<EnrollmentResponse> {
        self.exchange(&EnrollmentRequest::open_session(params)?)
            .await
    }

    async fn exchange(&self, request: &EnrollmentRequest) -> Result<EnrollmentResponse> {
        let value = exchange(&self.socket_path, request).await?;
        let response: EnrollmentResponse =
            decode_response(value, ENROLLMENT_RESPONSE_SCHEMA, ENROLLMENT_ERROR_SCHEMA)?;
        anyhow::ensure!(
            response.operation == request.operation,
            "PCP enrollment response operation does not match the request"
        );
        Ok(response)
    }
}

#[derive(Clone, Debug)]
pub struct EnrollmentAdminClient {
    socket_path: PathBuf,
}

impl EnrollmentAdminClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub async fn snapshot(&self) -> Result<EnrollmentAdminResponse> {
        self.exchange(&EnrollmentAdminRequest::snapshot()?).await
    }

    pub async fn approve(&self, request_id: impl Into<String>) -> Result<EnrollmentAdminResponse> {
        self.exchange(&EnrollmentAdminRequest::approve(request_id)?)
            .await
    }

    pub async fn reject(&self, request_id: impl Into<String>) -> Result<EnrollmentAdminResponse> {
        self.exchange(&EnrollmentAdminRequest::reject(request_id)?)
            .await
    }

    pub async fn revoke(
        &self,
        registration_id: impl Into<String>,
    ) -> Result<EnrollmentAdminResponse> {
        self.exchange(&EnrollmentAdminRequest::revoke(registration_id)?)
            .await
    }

    async fn exchange(&self, request: &EnrollmentAdminRequest) -> Result<EnrollmentAdminResponse> {
        let value = exchange(&self.socket_path, request).await?;
        let response: EnrollmentAdminResponse = decode_response(
            value,
            ENROLLMENT_ADMIN_RESPONSE_SCHEMA,
            ENROLLMENT_ADMIN_ERROR_SCHEMA,
        )?;
        anyhow::ensure!(
            response.operation == request.operation,
            "PCP enrollment admin response operation does not match the request"
        );
        Ok(response)
    }
}

fn validate_request_header(
    schema: &str,
    schema_version: &str,
    operation: &str,
    expected_schema: &str,
    expected_operation: &str,
) -> Result<()> {
    anyhow::ensure!(schema == expected_schema, "unsupported schema: {schema}");
    anyhow::ensure!(
        schema_version == PCP_ENROLLMENT_PROTOCOL_VERSION,
        "unsupported schema version: {schema_version}"
    );
    anyhow::ensure!(
        operation == expected_operation,
        "unexpected enrollment operation: {operation}"
    );
    Ok(())
}

async fn exchange(path: &Path, request: &impl Serialize) -> Result<Value> {
    let mut payload = serde_json::to_vec(request).context("serialize PCP enrollment request")?;
    payload.push(b'\n');
    anyhow::ensure!(
        payload.len() <= ENROLLMENT_MAX_REQUEST_FRAME_BYTES,
        "PCP enrollment request exceeds {ENROLLMENT_MAX_REQUEST_FRAME_BYTES} bytes"
    );

    let mut stream = timeout(ENROLLMENT_CLIENT_TIMEOUT, UnixStream::connect(path))
        .await
        .context("PCP enrollment connect timed out")?
        .with_context(|| format!("connect PCP enrollment socket {}", path.display()))?;
    timeout(ENROLLMENT_CLIENT_TIMEOUT, async {
        stream.write_all(&payload).await?;
        stream.flush().await
    })
    .await
    .context("PCP enrollment request write timed out")??;

    let mut response = Vec::new();
    let read = timeout(
        ENROLLMENT_CLIENT_TIMEOUT,
        BufReader::new(stream)
            .take(ENROLLMENT_MAX_RESPONSE_FRAME_BYTES as u64)
            .read_until(b'\n', &mut response),
    )
    .await
    .context("PCP enrollment response read timed out")?
    .context("read PCP enrollment response")?;
    anyhow::ensure!(
        read > 0,
        "PCP enrollment endpoint closed without a response"
    );
    anyhow::ensure!(
        response.last() == Some(&b'\n'),
        "PCP enrollment response did not end with LF"
    );
    anyhow::ensure!(
        response.len() <= ENROLLMENT_MAX_RESPONSE_FRAME_BYTES,
        "PCP enrollment response exceeds {ENROLLMENT_MAX_RESPONSE_FRAME_BYTES} bytes"
    );
    serde_json::from_slice(&response).context("decode PCP enrollment response")
}

fn decode_response<T: DeserializeOwned>(
    value: Value,
    response_schema: &str,
    error_schema: &str,
) -> Result<T> {
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .context("PCP enrollment response has no schema")?;
    let schema_version = value
        .get("schema_version")
        .and_then(Value::as_str)
        .context("PCP enrollment response has no schema_version")?;
    anyhow::ensure!(
        schema_version == PCP_ENROLLMENT_PROTOCOL_VERSION,
        "unsupported PCP enrollment response version: {schema_version}"
    );
    if schema == error_schema {
        let error: EnrollmentError =
            serde_json::from_value(value).context("decode PCP enrollment error")?;
        anyhow::bail!("{}: {}", error.code, error.message);
    }
    anyhow::ensure!(
        schema == response_schema,
        "unsupported response schema: {schema}"
    );
    serde_json::from_value(value).context("decode PCP enrollment response envelope")
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use pcp_core::{AccessPrincipalType, AccessSession};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixListener,
    };

    use super::*;

    #[test]
    fn begin_request_has_stable_wire_shape() {
        let request = EnrollmentRequest::begin(BeginEnrollmentParams {
            client: EnrollmentClientClaim {
                principal: EnrollmentPrincipalClaim {
                    principal_id: "symbiont".to_owned(),
                    principal_type: AccessPrincipalType::Host,
                    display_name: Some("Symbiont".to_owned()),
                },
            },
            requested_access: RequestedAccess {
                mode: RequestedAccessMode::Contribute,
                scopes: vec!["user:self".to_owned()],
                allow_cross_scope_derivation: false,
            },
            credential: "ab".repeat(32),
        })
        .unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["schema"], ENROLLMENT_REQUEST_SCHEMA);
        assert_eq!(value["operation"], "begin");
        assert_eq!(
            value["params"]["client"]["principal"]["principalType"],
            "host"
        );
        assert_eq!(value["params"]["requested_access"]["mode"], "contribute");
        assert_eq!(
            value["params"]["requested_access"]["scopes"][0],
            "user:self"
        );
    }

    #[test]
    fn active_response_round_trips() {
        let access = AccessSession::new(
            pcp_core::AccessPrincipal {
                principal_id: "host:symbiont-d".to_owned(),
                principal_type: AccessPrincipalType::Host,
                display_name: Some("Symbiont".to_owned()),
            },
            "enrolled:test",
            Vec::new(),
        );
        let response = EnrollmentResponse::new(
            "open_session",
            EnrollmentResult::Active {
                session: EnrollmentSession {
                    registration_id: "reg_test".to_owned(),
                    service: EnrollmentServiceIdentity {
                        kind: "pcp".to_owned(),
                        instance_id: "main".to_owned(),
                        generation: "proc_test".to_owned(),
                    },
                    binding: "infra.local.unix-socket".to_owned(),
                    endpoint: "sockets/pcp-session-test.sock".to_owned(),
                    access,
                },
            },
        );
        let encoded = serde_json::to_vec(&response).unwrap();
        let decoded: EnrollmentResponse = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let value = serde_json::json!({
            "schema": ENROLLMENT_REQUEST_SCHEMA,
            "schema_version": PCP_ENROLLMENT_PROTOCOL_VERSION,
            "operation": "status",
            "params": {},
            "extra": true
        });
        assert!(serde_json::from_value::<EnrollmentRequest>(value).is_err());
    }

    #[test]
    fn principal_claim_rejects_unknown_fields() {
        let value = serde_json::json!({
            "principalId": "host:symbiont-d",
            "principalType": "host",
            "displayName": "Symbiont",
            "executable": "/untrusted/path"
        });
        assert!(serde_json::from_value::<EnrollmentPrincipalClaim>(value).is_err());
    }

    #[tokio::test]
    async fn client_treats_response_lf_as_completion_without_waiting_for_eof() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = PathBuf::from("/tmp").join(format!("pcpe-wire-{}.sock", nonce % 1_000_000_000));
        let listener = UnixListener::bind(&path).expect("bind enrollment wire test socket");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept enrollment client");
            let (read, mut write) = stream.into_split();
            let mut request = Vec::new();
            BufReader::new(read)
                .read_until(b'\n', &mut request)
                .await
                .expect("read enrollment request");
            let response = EnrollmentResponse::new(
                "status",
                EnrollmentResult::Pending {
                    request_id: "req_test".to_owned(),
                    requested_at: "2026-08-10T12:00:00.000Z".to_owned(),
                    expires_at: "2026-08-10T12:05:00.000Z".to_owned(),
                },
            );
            write
                .write_all(&serde_json::to_vec(&response).unwrap())
                .await
                .expect("write enrollment response");
            write.write_all(b"\n").await.expect("write response LF");
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let client = EnrollmentClient::new(&path);
        let response = tokio::time::timeout(
            Duration::from_millis(250),
            client.status(EnrollmentStatusParams {
                request_id: "req_test".to_owned(),
                credential: "ab".repeat(32),
            }),
        )
        .await
        .expect("client must complete at LF")
        .expect("decode enrollment response");
        assert!(matches!(response.result, EnrollmentResult::Pending { .. }));
        server.abort();
        let _ = server.await;
        let _ = tokio::fs::remove_file(path).await;
    }
}
