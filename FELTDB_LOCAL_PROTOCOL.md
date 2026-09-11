# FeltDB Local Protocol v1: Implementation Guide

## Overview

This document specifies the Unix-domain-socket protocol between Combe (Rust) and FeltDB local runtime for durable workspace state persistence.

**Architectural Goal**: Enable native Rust applications to treat FeltDB as a language-independent durable state substrate.

**Current Status**: 
- ✅ Rust client complete (crates/combe-state)
- ✅ Protocol specification (RFC-style)
- ✅ Unix socket transport with framing
- ⏳ FeltDB runtime endpoint (requires @feltdb/core changes)

## Socket Transport

### Connection
- **Type**: Unix domain socket (UDS)
- **Location**: `$HOME/.local/share/combe/feltdb.sock` (or `XDG_DATA_HOME/combe/feltdb.sock`)
- **Permissions**: Owned by Combe process user, mode 0600
- **Lifecycle**: Created by FeltDB runtime, cleaned on graceful shutdown

### Message Framing

**Format**: Length-prefixed JSON

```
┌─────────┬─────────────────────┐
│  Size   │   JSON Message      │
│ 4 bytes │   variable length   │
│  LE int │                     │
└─────────┴─────────────────────┘
```

- **Size**: 32-bit little-endian unsigned integer (message byte count, not including size field)
- **Maximum message**: 10 MB (configurable)
- **Timeout**: 30s read, 10s write
- **Handling**:
  - Incomplete reads → timeout error
  - EOF during size read → "Runtime closed connection"
  - EOF during payload read → "Malformed message"
  - Oversized message → reject with error response

## Protocol Operations

### HELLO → READY Handshake

**Client initiates connection and sends HELLO request**:

```json
{
  "version": 1,
  "request_id": "uuid-string",
  "operation": "hello",
  "payload": {
    "client": "combe",
    "database": "combe"
  }
}
```

**Runtime responds with READY**:

```json
{
  "version": 1,
  "request_id": "same-uuid",
  "ok": true,
  "payload": {
    "version": 1,
    "protocol": "v1",
    "database": "combe",
    "runtime": "feltdb-js-0.10.0"
  }
}
```

**Validation**:
- Protocol version must be exactly 1
- Database identity must match request
- Client must reject incompatible versions
- Runtime may include additional capabilities in payload

### QUERY Operation

**Request**:

```json
{
  "version": 1,
  "request_id": "uuid-string",
  "operation": "query",
  "payload": {
    "collection": "workspaces",
    "filter": {
      "id": "workspace-id-or-empty-for-all"
    }
  }
}
```

**Response (success)**:

```json
{
  "version": 1,
  "request_id": "same-uuid",
  "ok": true,
  "payload": {
    "items": [
      {
        "id": "...",
        "kind": "folder",
        "path": "/home/user/project",
        "label": null,
        "position": 0
      }
    ]
  }
}
```

**Response (error)**:

```json
{
  "version": 1,
  "request_id": "same-uuid",
  "ok": false,
  "error": {
    "code": "COLLECTION_NOT_FOUND",
    "message": "Unknown collection: invalid"
  }
}
```

### TRANSACT Operation

**Request** (atomic multi-operation):

```json
{
  "version": 1,
  "request_id": "uuid-string",
  "operation": "transact",
  "payload": {
    "operations": [
      {
        "type": "upsert",
        "collection": "workspaces",
        "id": "new-workspace-id",
        "kind": "folder",
        "path": "/tmp/test",
        "label": null,
        "position": 0
      },
      {
        "type": "upsert",
        "collection": "tabs",
        "id": "new-tab-id",
        "workspace_id": "new-workspace-id",
        "title": "Tab Title",
        "position": 0,
        "focused_pane_id": null
      }
    ]
  }
}
```

**Response (success)**:

```json
{
  "version": 1,
  "request_id": "same-uuid",
  "ok": true,
  "payload": {
    "affected": 2
  }
}
```

**Key Properties**:
- All operations in one transaction commit atomically
- FeltDB own transaction semantics apply
- Partial failure is impossible (all-or-nothing)
- Each operation can be upsert, delete, or query
- Return `affected` count of modified records

## Error Codes

```
OK              ✓ Request succeeded
PROTOCOL_ERROR  Protocol version mismatch or malformed request
NOT_FOUND       Collection or entity not found (not an error for empty query)
INVALID_DATA    Document data fails validation or schema
TRANSACTION_FAILED  Transaction aborted (concurrent conflict, constraint violation)
RUNTIME_ERROR   Internal FeltDB error (should be rare)
```

## Request ID Semantics

Each request has a unique `request_id` (UUID v4). The runtime must:

1. **Associate** request_id with the transaction internally
2. **Return** the same request_id in response
3. **Log** diagnostically (client, operation, timestamp, outcome)
4. **Recover** across reconnects:
   - Client can query "what happened to request X?" if reconnect fails
   - Runtime must distinguish:
     - "Request not yet received"
     - "Request received, executed"
     - "Request received, failed with error Y"

## Lifecycle & Reconnect

### Initial Connection

```
Combe process starts
    ↓
Check: socket exists?
    ├─ yes → try connect
    └─ no  → launch runtime
                ↓
             wait for socket (timeout 30s)
                ↓
             connect
    ↓
send HELLO
    ↓
wait for READY
    ↓
ready for queries
```

### Reconnect After Socket Failure

```
Request fails with socket error
    ↓
Detect: runtime still running?
    ├─ yes → reconnect + HELLO/READY
    └─ no  → error (requires application recovery)
    ↓
For safe-to-retry queries: retry
For mutations: query first to check if executed
    ↓
if executed: return cached response
if not executed: retry
if uncertain: return error (application must retry)
```

## Collections & Schema (Combe v1)

FeltDB runtime must support these collections for Combe:

### workspaces
```typescript
{
  id: string;                // UUID
  kind: "home" | "repo" | "folder" | "worktree";
  path: string;              // Canonical filesystem path
  label?: string;            // Display name
  position: number;          // Sort order
}
```

### tabs
```typescript
{
  id: string;                // UUID
  workspace_id: string;      // FK to workspaces.id
  title: string;             // Display title
  position: number;          // Order within workspace
  focused_pane_id?: string;  // FK to panes.id
}
```

### panes
```typescript
{
  id: string;                // UUID
  tab_id: string;            // FK to tabs.id
  cwd: string;               // Current working directory
  split_parent_id?: string;  // FK to panes.id (for tree structure)
  split_direction?: "horizontal" | "vertical";
  split_ratio?: number;      // 0.0 to 1.0
  position: number;          // Order in parent split
}
```

### session_metadata
```typescript
{
  key: string;               // Session key (e.g., "selected_workspace_id")
  value?: string;            // JSON-encoded value
}
```

## Implementation Checklist for FeltDB Runtime

- [ ] Unix socket server on configured path
- [ ] Message size framing and timeout
- [ ] HELLO/READY handshake
- [ ] Protocol version validation
- [ ] Query operation with filter
- [ ] Transact operation (atomic multi-op)
- [ ] Error response structuring
- [ ] Request ID association and logging
- [ ] Collection schema validation
- [ ] Transaction atomicity
- [ ] Graceful shutdown (socket cleanup)
- [ ] Stale socket recovery
- [ ] Read/write timeout handling
- [ ] Malformed message rejection
- [ ] Oversized message rejection

## Testing Requirements

### Unit Tests (FeltDB side)
- Message serialization/deserialization
- Size framing edge cases
- Protocol version negotiation
- Error response formatting

### Integration Tests (Rust ↔ FeltDB)
- End-to-end: create workspace → tab → pane
- Persistence: write, close client, new client, read same state
- Restart: write, kill runtime, restart runtime, read state
- Migration: import state.json, verify in FeltDB
- Errors: malformed requests, oversized messages, timeouts
- Concurrent: multiple workspaces, rapid mutations

## Future Extensions (Post-v1)

Do not implement in v1:

- Subscriptions/streaming
- History/events
- Replication
- Remote databases
- Authentication
- Multi-database instances
- Generic administration
- Full FeltDB query language

These become v2 (or later) extensions.

## References

- Rust Transport: `crates/combe-state/src/transport.rs`
- Rust Protocol: `crates/combe-state/src/protocol.rs`
- Rust Client: `crates/combe-state/src/client.rs`
- Rust WorkspaceStore: `crates/combe-state/src/feltdb.rs`
