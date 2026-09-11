# AI continuity

Combe conversations are provider-neutral. Every request resolves a deterministic, bounded `COMBE_AI_CONTEXT v1`, fingerprints it, and records the actual provider, service, model, input, output, request identity, Work linkage, and usage associated with that context.

Context priority is current conversation, linked Work state, unresolved questions and decisions, relevant artifacts, and bounded relevant imported history. Historical material is reduced before current conversation or Work state. Unrelated imported conversations are excluded.

Provider failure cannot remove the persisted user message. Retrying reuses that message and creates a new provider attempt. A conversation may move between ChatGPT, Claude, local models, or later providers without changing identity.
