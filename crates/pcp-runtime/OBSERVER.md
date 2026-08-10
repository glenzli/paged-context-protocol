# PCP Runtime Observer Protocol

Protocol: `pcp.runtime.observer@20260810.1`

Status: draft

This document is the canonical application-protocol contract for the read-only
observer implemented by `pcp-runtime`. It defines the request, response, errors,
framing, and completion semantics. Infra Discovery only advertises where this
protocol is available.

The observer exposes aggregate operational state. It MUST NOT expose Page
content, query text, Scope names, raw audit events, storage paths, retention
actions, or maintenance controls. It is part of the reference Runtime control
plane, not the normative PCP Page model and not the PCP Console API.

## Discovery

Runtime advertises this protocol through
`infra.discovery.registration@20260810.1` with the
`infra.local.unix-socket` binding:

```json
{
  "schema": "infra.discovery.registration",
  "schema_version": "20260810.1",
  "service": {
    "kind": "pcp",
    "instance_id": "owner_...",
    "generation": "proc_..."
  },
  "lease": {
    "renewed_at": "2026-08-10T12:00:00.000Z",
    "expires_at": "2026-08-10T12:00:45.000Z"
  },
  "offers": [{
    "protocol": "pcp.runtime.observer",
    "protocol_versions": ["20260810.1"],
    "binding": "infra.local.unix-socket",
    "endpoint": "sockets/p....sock"
  }]
}
```

The Store's persistent owner ID is the stable `instance_id`. Each Runtime start
uses a new `generation` and a process-unique socket endpoint. The snapshot
identity MUST equal the selected registration's `kind`, `instance_id`, and
`generation`.

The runtime root, registration path, permissions, lease validation, atomic
replacement, publisher handoff, endpoint resolution, and consumer validation
follow the Infra Discovery specification. `INFRA_PROTOCOL_RUNTIME_DIR` may set
the final absolute runtime root. Otherwise the platform canonical root is used.
`PCP_OBSERVER_ENABLED=0` disables publication.

Runtime renews the 45-second lease every 15 seconds. One stable PCP identity has
one exclusive publisher. On shutdown, Runtime stops renewal and leaves the
stable manifest to expire naturally; it removes only its generation-specific
socket. `launchd` may supervise Runtime but is not a discovery mechanism.

## Binding And Trust Boundary

The endpoint is a runtime-root-relative `sockets/<opaque>.sock` path. Its parent
directories are owner-only and the socket mode is `0600`. Before reading any
application bytes, the provider obtains peer credentials from the connected
Unix stream and requires the peer effective UID to equal its own.

This protects the user boundary. It does not defend against a hostile process
already running as the same OS user. The protocol has no bearer token and the
registration contains no credential or Console URL.

## Framing

Each AF_UNIX stream connection carries exactly one request and one response:

1. The client writes one UTF-8 JSON object followed by LF.
2. The complete request frame, including LF, MUST NOT exceed 4096 bytes.
3. The server writes one UTF-8 JSON object followed by LF.
4. The complete response frame, including LF, MUST NOT exceed 1 MiB.
5. The response LF completes the message. The server then naturally closes the
   connection; there is no second request.

There is no length prefix, content negotiation, multiplexing, or multi-request
session. A peer-identity failure or a connection that times out before a complete
request may be closed without an application error. The server's request timeout
is five seconds and response-write timeout is ten seconds.

## Request

The only request in this version is this exact object plus LF:

```json
{"schema":"pcp.runtime.observer.request","schema_version":"20260810.1","operation":"snapshot"}
```

All three fields are required. Unknown fields, another operation, another schema,
invalid JSON, or an oversized frame are invalid requests.

## Snapshot

A successful request returns `pcp.runtime.observer.snapshot@20260810.1`:

```json
{
  "schema": "pcp.runtime.observer.snapshot",
  "schema_version": "20260810.1",
  "service": {
    "kind": "pcp",
    "instance_id": "owner_...",
    "generation": "proc_..."
  },
  "sequence": 1,
  "captured_at": "2026-08-10T12:00:01.000Z",
  "status": {
    "state": "healthy",
    "reason_codes": []
  },
  "headline_metrics": [
    "requests.total",
    "requests.latency.p95_ms",
    "pcp.pages.current"
  ],
  "metrics": [],
  "issues": [],
  "links": {
    "console_url": "http://127.0.0.1:4318/"
  },
  "extensions": {
    "pcp": {
      "protocol_version": "...",
      "capabilities": {},
      "integrity": {"state": "ok", "checked_at": "..."},
      "scope_count": 0,
      "health": {}
    }
  },
  "redaction": {
    "excluded": [
      "page_content",
      "query_text",
      "scope_names",
      "raw_audit",
      "storage_paths"
    ]
  }
}
```

`sequence` increases within one generation and has no cross-generation meaning.
`captured_at`, issue timestamps, and integrity timestamps are RFC 3339 values.
Status is `starting`, `healthy`, `degraded`, `unavailable`, or `stopping`.
`reason_codes` and issue `code` values are stable machine-readable identifiers;
issue severity is `info`, `warning`, or `critical`.

Each metric has `id`, `kind`, and a non-null `value`. `kind` is `gauge`,
`counter`, or `state`; `unit`, `window_seconds`, and `dimensions` are optional.
`headline_metrics` contains at most three IDs and MUST reference metrics present
in the same response.

| Metric ID | Kind | Unit | Window |
| --- | --- | --- | --- |
| `process.uptime_seconds` | gauge | seconds | current generation |
| `requests.total` | counter | calls | 86400 seconds |
| `requests.failed` | counter | calls | 86400 seconds |
| `requests.denied` | counter | calls | 86400 seconds |
| `requests.latency.p95_ms` | gauge | milliseconds | 86400 seconds; optional |
| `requests.telemetry_coverage_ratio` | gauge | ratio | 86400 seconds; optional |
| `pcp.pages.current` | gauge | pages | current Store |

Latency and telemetry coverage are omitted when unknown; they are never emitted
as JSON `null`. Snapshot polling is excluded from workload metrics.

`extensions.pcp.capabilities` and `extensions.pcp.health` use the PCP Runtime's
existing aggregate DTOs. Per-Scope health rows are always removed and `health`
is omitted if aggregation fails. `integrity` is cached and refreshed no more
often than every ten minutes. A pending first check yields `starting`; failed
integrity or Health aggregation yields `degraded`. The provider does not infer
infrastructure status from Page semantics.

`links.console_url` is optional and comes only from
`PCP_OBSERVER_CONSOLE_URL`; it is a presentation deep link. It is not discovery,
and its presence does not make Console DTOs part of this protocol.

## Errors

After accepting a valid same-user connection, the provider may return:

```json
{"schema":"pcp.runtime.observer.error","schema_version":"20260810.1","code":"invalid_request","message":"..."}
```

Defined codes are `invalid_request`, `snapshot_unavailable`, and
`response_too_large`. `message` is diagnostic text and is not a stable matching
surface. Error responses obey the same one-line size and close semantics.

## Minimal Python Adapter

After Infra Discovery has validated and selected the registration and offer, a
Sentinel adapter needs only this application client:

```python
import json
import socket
from pathlib import Path

REQUEST = {
    "schema": "pcp.runtime.observer.request",
    "schema_version": "20260810.1",
    "operation": "snapshot",
}

def snapshot(runtime_root: Path, endpoint: str) -> dict:
    socket_path = runtime_root / endpoint
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(5)
        client.connect(socket_path)
        client.sendall(json.dumps(REQUEST, separators=(",", ":")).encode() + b"\n")
        with client.makefile("rb") as response:
            payload = response.readline(1024 * 1024 + 1)
            trailing = response.read(1)
    if not payload.endswith(b"\n") or len(payload) > 1024 * 1024 or trailing:
        raise ValueError("invalid PCP observer response frame")
    value = json.loads(payload)
    if value.get("schema") == "pcp.runtime.observer.error":
        raise RuntimeError(value)
    if value.get("schema") != "pcp.runtime.observer.snapshot":
        raise ValueError("unexpected PCP observer response schema")
    if value.get("schema_version") != "20260810.1":
        raise ValueError("unsupported PCP observer response version")
    return value
```

The discovery layer remains responsible for validating the runtime root,
manifest ownership and permissions, lease, exact protocol version, binding,
relative endpoint, and socket ownership before this adapter connects.
