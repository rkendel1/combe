# ChatGPT history import

Combe acquires history only from an official ChatGPT data export selected by the user. This is local, user-mediated data acquisition, not ChatGPT account access. Combe does not request credentials, cookies, session tokens, or OAuth access and does not upload the export.

## Studio

Choose **Work → Import ChatGPT History…**, select the export ZIP, then search and date-filter the discovered conversations. Nothing is written during discovery. Select the conversations explicitly and confirm the import. The completion result distinguishes new conversations, updated conversations, new messages, duplicates, and skipped records.

## CLI

```sh
combe chatgpt import ~/Downloads/chatgpt-export.zip --dry-run
combe chatgpt import ~/Downloads/chatgpt-export.zip --search "canonical context" --from 2025-01-01
combe chatgpt import ~/Downloads/chatgpt-export.zip --conversation <source-id> --json
```

`--dry-run` performs archive discovery, parsing, normalization, and selection without opening or mutating Combe state. The CLI and Studio use the same importer and reconciliation path.

## Storage and continuity

Selected messages become `COMBE_AI_CONTEXT` conversation events. Each retains `source = chatgpt_export`, the import ID, original conversation and message IDs, source timestamp, import timestamp, branch relationships, and attachment references. A small ChatGPT-specific index supports idempotent reconciliation and source management; it is not authoritative conversation state.

Re-importing the same material creates no duplicate conversation or message. A later export adds unseen messages to the stable `chatgpt:<source_conversation_id>` canonical conversation. Imported history is contextual evidence. It cannot create or revise Work decisions, assignments, executions, or other authoritative Work state.

The context resolver performs bounded term relevance lookup. It excludes the current conversation and does not include unrelated imported history merely because it is available.
