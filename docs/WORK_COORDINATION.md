# Work coordination

A Work is durable coordination state, not a transcript. Its current projection answers what is being considered, what was reviewed, what was approved, what is executing, and what resulted without replaying a provider conversation.

## Records

- A Turn is a bounded thing a participant said or did.
- A Proposal is something a participant suggests for consideration. It may retain local or external-conversation provenance.
- A Proposal review is an explicit evaluation: approve, reject, or request changes.
- A Decision is the authoritative conclusion created when a Proposal is approved.
- An Assignment is authorized work for a participant to perform. When linked to a Proposal, that Proposal must already be approved.
- An Execution is one durable provider-neutral materialization of an Assignment.
- An execution result is the bounded outcome of that Execution.
- An Artifact is repository-owned evidence or output of performed work.

Proposal, review, and decision are separate because conversational agreement is not authorization. Approval atomically persists the review, changes the Proposal to Approved, and creates its Decision. Rejection atomically persists the review and changes the Proposal to Rejected. Requesting changes persists the review while leaving the Proposal Proposed.

## Lifecycle

Draft Proposals may be prepared before they are Proposed. Review operations apply only to Proposed records. Approve and Reject are terminal; Superseded records are retained but inactive. An approved Proposal may be linked to an Assignment. The existing handoff completion transaction then records assignment status, result, Turn, and Artifacts.

The preserved chain is:

`Proposal → Proposal review → Decision → Assignment → Execution → result/Turn → Artifacts`

An Assignment remains Pending until execution starts. Start atomically creates its sole canonical Execution and makes the Assignment Active. Completion, failure, or cancellation is an explicit fenced terminal transition. Provider IDs are external references; Work IDs and Execution IDs remain canonical.

## External participants

External AI conversations remain external. Combe stores bounded contributions, references, and explicit provenance, never provider history or credentials. Provider integration remains an adapter concern; the Work domain has no OpenAI, Anthropic, Codex, or other provider dependency.

## CLI workflow

Create a Proposal by piping its statement:

```sh
cat proposal.md | combe work proposal create WORK_ID --participant ChatGPT --title "Move authorization into AuthPort" --conversation CHAT_ID
```

Review it explicitly:

```sh
combe work proposal approve PROPOSAL_ID --by Human
```

Create an authorized assignment:

```sh
combe work assign WORK_ID --to Codex --proposal PROPOSAL_ID --instruction "Implement the approved approach"
```

Exported `COMBE_WORK_CONTEXT` version 2 contains the bounded coordination projection for the next participant.

The version 2 package is also the provider-neutral Work protocol. Each package carries the FeltDB authority revision on which it is based. Returned contributions must identify that revision; stale submissions are rejected rather than merged. Proposal reviews use `ProposalReview`, while reviews of completed or failed work use an `ExecutionReview` anchored to its Execution and Assignment.
