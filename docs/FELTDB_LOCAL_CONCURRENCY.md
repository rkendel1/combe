# FeltDB local concurrency

Combe's FeltDB file has one live process owner. Clones and repeated opens inside that process share the same mutex-backed state. `FeltDb::open()` from another process fails with `WouldBlock` while the owner is alive.

## Verified source behavior

Before the ownership change, `FeltDb::open()` replayed the complete JSONL operation log into a handle-local in-memory snapshot. `Arc<Mutex<Inner>>` serialized clones of one handle, but separately opened handles had unrelated mutexes and snapshots. They could append to the same file concurrently, retain stale reads, evaluate transaction preconditions against different histories, and assign conflicting sequence and authority revisions. There was no OS file lock.

The durable log scanner treats an unterminated final record as a torn append and truncates it after otherwise successful replay. A concurrent opener could therefore classify another process's in-progress append as crash debris. Concurrent direct opens were not safe for readers, writers, operation-log writes, or atomic transactions.

`FeltDb::open()` canonicalizes the database location, then reuses a process-local weak registry entry for repeated opens, so aliases cannot create independent owners and every handle in one process shares the same state, events, and ownership file. The first open takes a non-blocking exclusive OS lock on the database's sibling `.lock` file before format inspection, replay, recovery truncation, or creation. The open file remains in the shared `FeltDb` value for its lifetime. The lock is advisory: all access through `FeltDb::open()` participates, while a process that writes database bytes directly can still violate the contract.

## Safety boundaries

| Property | Contract |
| --- | --- |
| Thread safety | Clones and same-process opens share an `Arc<Mutex<Inner>>`; operations are serialized in that owner. |
| Process safety | Exactly one process may own a database path. Another open fails clearly; concurrent multi-process reads and writes are unsupported. |
| File locking | A non-blocking exclusive OS lock is held on the sibling `.lock` file for the owner's lifetime. |
| Transaction atomicity | An atomic transaction is one newline-terminated log record, synchronized with `sync_data()` before in-memory publication. Atomicity applies inside the sole-owner contract. |
| Single-record durability | The default path writes one newline-terminated record and flushes it to the operating system. It does not establish power-loss durability. |
| Crash safety | Replay accepts complete valid records, discards and reports one unterminated tail, and fails closed on malformed terminated records. Transaction records are recovered whole or discarded as an incomplete tail. |
| Recovery ownership | FeltDB owns log validation, replay, tail recovery, and durable-format compatibility. Combe does not repair or reinterpret the log. |

## Combe ownership

Within one Combe process, workspace and Work stores obtain the same shared `FeltDb` handle. FeltDB also enforces same-process reuse at its public open boundary. Combe owns the database location and its `combe/*` schema conventions; it does not add a lock around individual calls.

The GUI releases its short-lived store handle before injecting `combe work handoff` into Ghostty. The handoff CLI then owns FeltDB while it records assignment transitions and the result. GUI Work operations attempted during that interval fail to open rather than race or corrupt the log. No existing local IPC mechanism can route these commands through the GUI, so no database protocol or proxy was added.

PTYs and external agent processes remain ephemeral. Work and FeltDB records are durable. Agents operate on repository files and do not open Combe's database directly.

## Regression evidence

`feltdb/tests/local_process_ownership.rs` launches a second OS process against the same real database. It proves that a second reader/writer/transaction owner is rejected while the first owner lives, that ownership is released on process exit, and that the next owner can recover the committed transaction.

FeltDB's existing recovery tests cover torn-tail recovery and transaction crash atomicity. Killing a process inside the single `write_all` call is not a controllable public test boundary, so this change does not claim a new power-loss guarantee.
