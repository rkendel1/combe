# External conversation context

Combe Work is the durable coordination context. External AI conversations remain owned by their providers. Combe stores provider-neutral references and bounded contributions, not mirrored conversation histories.

Imported conversation material may support a Proposal, but it never constitutes approval. A Proposal records its external conversation provenance when supplied; a participant must still perform an explicit review operation before Combe creates an authoritative Decision.

## Model

A Participant identifies who contributes to a Work. A Conversation reference identifies where an external conversation lives. A Work conversation links the two without assuming the provider's identifier format. Imported content becomes a normal bounded Work Turn with external-conversation provenance.

FeltDB remains the sole durable authority for Work metadata. Conversation links use `combe/work-conversation/{id}`. Repository files remain authoritative for artifact contents. No browser, provider credential, remote API, transcript mirror, polling process, or shadow database participates.

## Manual ChatGPT workflow

1. Create or select a Work.
2. Export its context with `combe work context <work-id> --format text`.
3. Give that envelope to ChatGPT manually.
4. Copy the relevant response.
5. Import it with `combe work import <work-id> --participant ChatGPT --provider chatgpt --conversation <conversation-id> --title "Architecture discussion" --kind analysis` and provide the copied text on stdin.
6. Create an assignment for Codex or Claude and run the existing handoff.
7. Export the updated Work context for the next participant.

Later contributions to an already linked conversation may use the same import command. Locally authored material uses `combe work contribute <work-id> --participant <participant> --kind <kind>`.

## Canonical export

Text exports begin with `COMBE_WORK_CONTEXT`, declare `version: 2`, include the authority revision and requested participant action, contain the bounded Work projection and repository identity, and end with `END_COMBE_WORK_CONTEXT`. JSON exports serialize the same `ContextPackage`. Neither representation exposes unrelated FeltDB records or repository contents.

Provider integration belongs behind adapters. The Work domain has no dependency on OpenAI, Anthropic, Codex, ChatGPT internals, or any provider conversation protocol.

External participants exchange bounded `WorkRequest`, `WorkContext`, and `WorkContribution` protocol values. Their contribution retains the external Conversation reference and the FeltDB revision of the context they consumed. Provider integrations translate these values; they never mirror conversations or own a parallel Work model.
