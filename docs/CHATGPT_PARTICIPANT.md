# ChatGPT participant adapter

ChatGPT is a Work Participant, not a Work authority, database owner, execution engine, or source of truth. The adapter translates between provider-native conversations and the provider-neutral Combe Work protocol.

## Current transport

| Capability | Status |
| --- | --- |
| Direct transport | Unavailable |
| Manual context transfer | Supported |
| Structured response import | Supported |
| Credential persistence | None |

Combe does not use an OpenAI API, browser automation, cookies, session tokens, scraping, or hidden network calls. The user explicitly transfers context and imports a response.

## Commands

Prepare bounded canonical context:

```sh
combe work chatgpt <work-id> --action inspect
combe work chatgpt <work-id> --action propose
combe work chatgpt <work-id> --action review
```

An optional `--conversation <external-id>` and `--title <title>` link a real provider-owned conversation. When no identifier is available, Combe does not invent one.

Import copied output:

```sh
pbpaste | combe work chatgpt import <work-id> --kind message
pbpaste | combe work chatgpt import <work-id> --kind proposal --title "Approach"
pbpaste | combe work chatgpt import <work-id> --kind review --execution <execution-id>
```

Plain text is preserved under the explicitly selected kind. JSON may use the documented version 2 response envelope. Imports retain `based_on_revision`; stale responses are rejected without mutation. A ChatGPT review may recommend approval, but only a Participant with decide capability authorizes Work.
