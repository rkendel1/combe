# Provider credential boundary

Credential contents exist only inside the credential boundary and macOS Keychain. Serializable Provider profiles contain an optional `CredentialRef`, never a secret. Work messages and provider-neutral results contain routing provenance but no authorization material.

The OpenAI adapter retrieves its bearer credential immediately before the HTTPS request and never logs or serializes the header. Ollama accepts only a configured loopback HTTP endpoint. Claude Code inherits its own CLI authentication. ChatGPT manual transfer has no credential integration.

Tests use an in-memory implementation behind `CredentialStore`; production has no plaintext or fake credential store. Integration tests use real providers only when explicitly configured and never commit credentials.
