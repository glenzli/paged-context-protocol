# PCP Runtime Enrollment Protocol

Protocol: `pcp.runtime.enrollment@20260810.1`

Status: draft

This document is the canonical application-protocol contract for discovering a
local PCP Runtime, requesting user-approved access, and reopening an approved
identity-bound PCP RPC session. Infra Discovery advertises the public entry
point; it does not define enrollment requests, responses, credentials, approval,
or the PCP RPC session returned after approval.

Enrollment is part of the official Runtime control plane, not the normative PCP
Page model. Static configured endpoints remain supported and can coexist during
migration.

## Discovery

Runtime advertises enrollment through
`infra.discovery.registration@20260812.1` using the
`infra.local.unix-socket` binding. The observer and enrollment protocols may
share one generation-specific endpoint:

```json
{
  "schema": "infra.discovery.registration",
  "schema_version": "20260812.1",
  "service": {
    "kind": "pcp",
    "instance_id": "idn_...",
    "generation": "proc_..."
  },
  "offers": [
    {
      "protocol": "pcp.runtime.observer",
      "protocol_versions": ["20260810.1"],
      "binding": "infra.local.unix-socket",
      "endpoint": "sockets/7K2M9Q4V6W8X1Y3Z.sock"
    },
    {
      "protocol": "pcp.runtime.enrollment",
      "protocol_versions": ["20260810.1"],
      "binding": "infra.local.unix-socket",
      "endpoint": "sockets/7K2M9Q4V6W8X1Y3Z.sock"
    }
  ]
}
```

The Store `identityId` is the stable `instance_id`. Every Runtime start has a new
`generation` and public socket. A client MUST select an exact supported protocol
version and a valid relative endpoint according to the Infra Discovery
specification. The manifest is only a candidate declaration; connection success,
not its presence or modification time, establishes endpoint availability. After
a connection failure, the client MUST rescan before retrying, and it MUST
rediscover after a generation change instead of retaining the public socket path.

`PCP_ENROLLMENT_ENABLED=0` disables the enrollment offer and service. It does not
disable the observer offer. `INFRA_PROTOCOL_RUNTIME_DIR` has the same meaning as
for the observer.

## Trust And Credential Model

Before reading application bytes, every public, administration, or dynamic PCP
RPC endpoint requires the peer effective UID to equal the Runtime UID. All
sockets are mode `0600`.

The client generates 32 random bytes and persists them as 64 lowercase
hexadecimal characters. The credential is sent in `begin`, `status`, and
`open_session`; Runtime persists only its SHA-256 digest. A credential is not a
requested PCP Identity or permission. Runtime binds every approved registration
to the Identity advertised by the selected PCP service and constructs the final
`AccessSession` exclusively from a user-approved registration.

The public endpoint permits applications to create and inspect only requests
whose credential they possess. Approval, rejection, registration listing, and
revocation use a separate, undiscovered administration socket. PCP Console is
the reference approval UI.

This is a same-user consent and accidental-isolation boundary. It does not
protect against a hostile process already running as the same OS user, which can
connect to owner-only local sockets or imitate another application's display
claim. Strong isolation requires an OS-level identity boundary.

## Framing

Public and administration endpoints use the same framing:

1. One AF_UNIX connection carries exactly one request and one response.
2. The request is one UTF-8 JSON object followed by LF.
3. The complete request frame, including LF, MUST NOT exceed 16 KiB.
4. The response is one UTF-8 JSON object followed by LF.
5. The complete response frame, including LF, MUST NOT exceed 128 KiB.
6. The response LF completes the message; the server then naturally closes the
   connection.

There is no length prefix, multiplexing, or multi-request stream. A peer failure,
timeout, or incomplete frame may close without an application error.

## Public Requests

Every public request has schema `pcp.runtime.enrollment.request` and version
`20260810.1`. Unknown envelope or operation-parameter fields are rejected.

### Begin

```json
{
  "schema": "pcp.runtime.enrollment.request",
  "schema_version": "20260810.1",
  "operation": "begin",
  "params": {
    "client": {
      "principal": {
        "principalId": "host:symbiont-d",
        "principalType": "host",
        "displayName": "Symbiont"
      }
    },
    "requested_access": {
      "mode": "contribute",
      "scopes": [
        "user:self",
        "project:symbiont-d",
        "conversation:symbiont-d"
      ],
      "allow_cross_scope_derivation": false
    },
    "credential": "<64 lowercase hexadecimal characters>"
  }
}
```

`principalType` is `host`, `model_client`, `cli`, or `service`. `mode` is
`observe`, `read`, `contribute`, `audit`, `write`, `repair`, or `admin`. Ordinary tenants
use `read` or `contribute`; `contribute` adds only authenticated `ingest_page`
and does not grant advanced Page or maintenance writes. `write` and `admin` are
privileged maintainer and local-operator modes. `repair` is a narrow,
approval-gated development migration mode: it adds `repair_page` to normal
read/search access but grants neither `ingest_page`, ordinary `write_page` /
`revise_page`, nor lifecycle or Scope management. Use a separate Principal and
credential, and open that registration only while applying an explicit repair
migration. The special Scope `user:self` is
resolved by Runtime to the selected Store's `user:<identity_id>` Scope. Other
Scope names are literal. A request contains 1-16 unique Scopes, each at most 128
UTF-8 bytes. Runtime retains at most 16 simultaneous pending requests and 32
active registrations per Store.

`begin` is idempotent for the same credential, client claim, and requested
access. Before approval it returns `pending`; after approval it may return an
active session directly.

### Status

```json
{
  "schema": "pcp.runtime.enrollment.request",
  "schema_version": "20260810.1",
  "operation": "status",
  "params": {
    "request_id": "req_...",
    "credential": "<same credential>"
  }
}
```

The client may poll `status` while waiting for user action. A pending request
expires after five minutes by default. Approved requests remain recoverable
while their registration is active.

### Open Session

```json
{
  "schema": "pcp.runtime.enrollment.request",
  "schema_version": "20260810.1",
  "operation": "open_session",
  "params": {
    "registration_id": "reg_...",
    "credential": "<same credential>"
  }
}
```

Clients use `open_session` after every Runtime generation change. The returned
endpoint is generation-specific and MUST NOT be used with a different selected
discovery registration.

## Public Responses

Successful responses use `pcp.runtime.enrollment.response@20260810.1`, echo the
operation, and contain one tagged result.

Pending:

```json
{
  "schema": "pcp.runtime.enrollment.response",
  "schema_version": "20260810.1",
  "operation": "begin",
  "result": {
    "status": "pending",
    "request_id": "req_...",
    "requested_at": "2026-08-10T12:00:00.000Z",
    "expires_at": "2026-08-10T12:05:00.000Z"
  }
}
```

Rejected:

```json
{
  "schema": "pcp.runtime.enrollment.response",
  "schema_version": "20260810.1",
  "operation": "status",
  "result": {"status": "rejected", "request_id": "req_..."}
}
```

Active:

```json
{
  "schema": "pcp.runtime.enrollment.response",
  "schema_version": "20260810.1",
  "operation": "open_session",
  "result": {
    "status": "active",
    "session": {
      "registration_id": "reg_...",
      "service": {
        "kind": "pcp",
        "instance_id": "owner_...",
        "generation": "proc_..."
      },
      "binding": "infra.local.unix-socket",
      "endpoint": "sockets/4F6H8J1K3M5N7P9Q.sock",
      "access": {
        "principal": {
          "principalId": "host:symbiont-d",
          "principalType": "host",
          "displayName": "Symbiont"
        },
        "sessionId": "enrolled:reg_...:proc_...",
        "grants": []
      }
    }
  }
}
```

The client resolves `endpoint` against the selected Infra Protocol runtime root,
connects with the existing PCP RPC transport, and verifies that the RPC
descriptor's `access` equals the returned `access`. The session identity's
`kind`, `instance_id`, and `generation` MUST exactly match the selected discovery
registration. Dynamic endpoints use the same canonical
`sockets/<opaque>.sock` binding shape as discovery offers: the opaque ID is at
most 16 filename-safe ASCII characters, and Runtime validates the final path
before binding and retries with a new ID on collision.

The PCP RPC descriptor publishes one `identityId`, which MUST equal the selected
service `instance_id`. A client MUST reject a mismatch.

## Administration

The administration socket defaults to `pcp-enrollment-admin.sock` beside the
configured static Runtime endpoint. `PCP_ENROLLMENT_ADMIN_SOCKET` overrides it.
It is not advertised by Infra Discovery.

Requests use `pcp.runtime.enrollment.admin.request@20260810.1`:

```json
{"schema":"pcp.runtime.enrollment.admin.request","schema_version":"20260810.1","operation":"snapshot","params":{}}
{"schema":"pcp.runtime.enrollment.admin.request","schema_version":"20260810.1","operation":"approve","params":{"request_id":"req_..."}}
{"schema":"pcp.runtime.enrollment.admin.request","schema_version":"20260810.1","operation":"reject","params":{"request_id":"req_..."}}
{"schema":"pcp.runtime.enrollment.admin.request","schema_version":"20260810.1","operation":"revoke","params":{"registration_id":"reg_..."}}
```

Responses use `pcp.runtime.enrollment.admin.response@20260810.1`. `snapshot`
returns `result.status = "snapshot"` with `pending` and `registrations`; mutation
responses return `result.status = "applied"`. Views contain client claims,
requested or approved access, IDs, and timestamps, but never credential hashes.
Revocation closes an active dynamic endpoint and prevents later reopening.

## Errors

Public errors use `pcp.runtime.enrollment.error`; administration errors use
`pcp.runtime.enrollment.admin.error`:

```json
{
  "schema": "pcp.runtime.enrollment.error",
  "schema_version": "20260810.1",
  "code": "not_found",
  "message": "enrollment is not available"
}
```

Stable public codes are `invalid_request`, `not_found`, `session_unavailable`,
`capacity_exceeded`, `response_too_large`, and `internal_error`. `message` is
diagnostic and not a matching surface. A bad credential and an unknown or
revoked ID both return `not_found`.

## Symbiont Migration

Symbiont can migrate without a coordinated flag day:

1. Keep its current configured PCP Runtime socket as a temporary fallback.
2. Discover a compatible `pcp.runtime.enrollment@20260810.1` offer.
3. Generate and durably store one enrollment credential; call `begin` with the
   existing `host:symbiont-d` claim and current Scope request.
4. Poll `status` until PCP Console approves or rejects the request.
5. Persist `registration_id`, resolve the returned session endpoint, verify its
   discovery identity and RPC descriptor, and use the existing PCP RPC client.
6. On disconnect or generation change, rediscover and call `open_session`; do not
   persist a Runtime socket path.
7. Remove the static-socket fallback only after an approved registration has
   reopened successfully across a Runtime restart.

Enrollment state defaults to `pcp-enrollments.json` beside the Store and can be
overridden with `PCP_ENROLLMENT_STATE_PATH`. Runtime writes it atomically as mode
`0600`; it contains credential digests, requests, decisions, and registrations,
but no Page content or plaintext credential.
