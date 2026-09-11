# Work coordination protocol

Combe Work is the coordination protocol and durable state. AI providers are participants, not owners of the Work. A provider integration translates between its native conversation or action model and this protocol; it must not create a parallel Work model.

## Package

`COMBE_WORK_CONTEXT` version 2 has one canonical JSON representation. The text form renders the same package for humans and language models. A package contains a `WorkRequest`, one bounded `WorkContext`, repository identity, and constraints.

The request requires `work_id`, `participant_id`, `context_version`, `revision`, and `requested_action`. Version 2 actions are Inspect, Propose, Review, Decide, Assign, Execute, and Report. Identifiers are opaque canonical strings. Provider execution IDs and Conversation references are external provenance, never canonical identity.

The context contains current objective and status; Participants and capabilities; active and approved Proposals; Proposal reviews and Decisions; bounded Assignments, Executions, results, Artifact references, Conversation links, Execution reviews, and recent Turns; and derived current-state groups. Lists use creation time then canonical ID ordering unless they are explicitly bounded recent projections. The limits are compiled constants in `WorkStore`.

## Contributions

A `WorkContribution` requires its Work and Participant IDs, protocol version, the FeltDB revision it consumed, source, creation time, kind, and bounded content. It may reference a Proposal, Assignment, and Execution when the contribution concerns those records. Kinds are Message, Proposal, Review, Decision, ExecutionReport, and ArtifactReport.

Proposal contributions become `WorkProposal`. Reviews with a Proposal reference become `ProposalReview`. Reviews with an Execution reference become `ExecutionReview`; the Assignment reference must match that Execution. Decisions use the existing `WorkDecision`. Execution reports use the canonical Execution transition API rather than creating a second record.

## Validation and concurrency

Combe validates version, capability, Work membership, source provenance, bounds, and every referenced relationship before accepting a contribution. It never repairs malformed input. Acceptance uses the existing FeltDB atomic transaction machinery with the package revision as its expected authority revision. A stale revision returns the expected and current revision and writes nothing. The participant must fetch a fresh package and retry; there is no automatic merge.

External conversations remain provider-owned. Work retains only bounded contributions and explicit Conversation provenance. JSON ordering and serialization are deterministic for the same canonical state and revision.
