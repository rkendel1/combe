# Provider registry and recipient routing

Every outbound Work message names a Recipient. A Recipient references one durable Provider profile; the profile identifies the service, model, capabilities, execution mode, endpoint where applicable, and an optional non-secret credential reference.

```text
Work → Recipient → Provider profile → Provider adapter → Transport → Provider
```

Ollama uses a configured local HTTP endpoint and explicit model. Claude Code uses the `claude` executable resolved through `PATH` and an explicit worktree. ChatGPT remains an external consumer conversation with manual context transfer. OpenAI API is a separate service using the Responses API and a Keychain credential.

The router invokes only the selected Recipient's profile. Missing services, models, credentials, executables, endpoints, or worktrees fail explicitly. No adapter falls back to another provider.

Provider profiles, Recipients, outbound messages, and bounded provider-neutral results persist in FeltDB. Results retain the selected recipient, profile, model, service, execution mode, and originating message across restart. Credential bytes are stored by the macOS Keychain adapter under service `com.randy.combe.provider-credential`; FeltDB stores only the account-like credential reference. CLI inspection never prints credential contents.

Provider configuration is deliberately narrow. It is not a general settings system, plugin mechanism, autonomous scheduler, or provider marketplace.
