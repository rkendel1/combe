# Work execution

An Assignment authorizes work. An Execution records what actually ran. Work owns the canonical lifecycle; a replaceable provider adapter owns only execution mechanics.

Each Assignment has at most one canonical Execution. Start atomically creates it and changes the Assignment from Pending to Active. A started Execution may atomically become Completed, Failed, or Cancelled. Completion and failure retain a bounded result; cancellation retains its explicit reason. Terminal transitions update the Assignment and Execution together under FeltDB version preconditions.

Provider names and provider execution IDs are external references. They never replace canonical Work, Assignment, or Execution identity. Provider output is bounded in Work context and raw transcripts remain outside FeltDB.

Heartbeat updates liveness metadata without changing status. Active or stale is derived from the last heartbeat and the compiled stale interval. Combe does not schedule heartbeats, retry providers, or recover work automatically.

If a provider crashes or times out without reporting a terminal observation, the Execution remains Started and becomes stale. A human or provider may then explicitly fail or cancel it; Combe never invents the outcome.

The preserved chain is:

`Conversation → Proposal → Proposal review → Decision → Assignment → Execution → result/Turn → Artifacts`

The CLI exposes `work start`, `work status`, `work heartbeat`, `work complete`, `work fail`, and `work cancel`. Installed Codex and Claude handoffs use the same canonical transitions as every other provider.

A participant may review a terminal Execution through the Work protocol. The resulting Execution review identifies its Work, Assignment, Execution, reviewer, source, and originating context revision. It is evidence for a later Proposal or Decision, not automatic approval.
