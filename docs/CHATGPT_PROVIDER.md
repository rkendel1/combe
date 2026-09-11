# ChatGPT provider

ChatGPT is a replaceable provider for a Combe conversation. Combe owns conversation identity and messages, FeltDB owns durable state, Keychain owns the API credential, and `COMBE_AI_CONTEXT` defines exactly what is sent.

A ChatGPT provider profile uses a configured model, HTTP API execution, and a Keychain credential reference. The model is not hard-coded. Changing the profile or model does not change the Combe conversation ID. Manual ChatGPT transfer remains available as a separate execution mode and does not impersonate a provider response.

The native runtime uses the OpenAI Responses API with streaming enabled and storage disabled. Response deltas are transient. Completion creates one canonical assistant message. A failed attempt retains bounded partial output and sanitized provenance but no fake completed response.

Combe does not access a private ChatGPT account, web history, login, OAuth session, cookie, or subscription. ChatGPT web authentication is unrelated to API authentication.
