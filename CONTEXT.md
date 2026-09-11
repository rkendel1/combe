# Combe terms

Use these words in code, commits, and docs. If a new domain word sticks, add it here instead of inventing a synonym.

| Term | Meaning |
| --- | --- |
| Combe | Product name. A valley you sit in. |
| Repo | A filesystem path the user registered. Not discovered by walking the disk. |
| Worktree | One row from `git worktree list --porcelain` for a registered git repo. |
| Folder workspace | A registered path that is not a git checkout. Shown as a single row. |
| Workspace | The selected worktree, folder workspace, or Home workspace. The shell cwd of a tab. |
| Home workspace | Built-in folder workspace at `$HOME`. Not a registered repo. Forced folder even if `$HOME` is a git checkout. Injected first in the sidebar catalog unless a catalog row already owns that path. Label `~`. |
| Catalog | The merge of `state.json` + git porcelain + folder fallback + the Home workspace when `$HOME` is not already a row. Missing registered paths are skipped. |
| Sidebar | The catalog shown as a workspace chip, transient panel, or pinned sidebar. ⌘B toggles pinned and chip states. |
| Session mark | Sidebar dot on a worktree row. Green when that workspace has a tab this app run; dim when it does not. |
| Tab | One split tree, opened on one workspace. |
| Pane | One node of a tab's split tree. Either an `NSSplitView` or a surface. |
| Surface | One libghostty surface: its own Metal layer, PTY, VT state, and font stack. The leaf of a split tree. |
| Split | A cut that reparents the focused surface into a new pane beside a fresh sibling. |
| Chrome | Everything AppKit draws: window, sidebar, tab bar, status line, splits. Never the terminal. |
| Status line | Bottom chrome row on the right pane. Present only while a quota chip is shown. |
| Quota | One status-line block of locally recorded CLI subscription snapshots. Each name is followed by remaining percent of its tightest 5h/7d window. Hover or activation expands the chip upward with every available provider's windows. |
| Habits | Compiled-in preferences in `crates/combe/src/habits.rs`. There is no config file. |
| Occlusion | A hidden tab's surfaces are told to stop drawing via `ghostty_surface_set_occlusion`. |
| State file | `~/Library/Application Support/combe/state.json` |
| Work | A durable unit of human and external-agent collaboration belonging to a workspace. Execution is not part of its identity. |
| Participant | A provider-neutral human, agent, or system contributing to a Work. |
| Turn | One durable contribution to a Work, independent of a terminal invocation or provider transcript. |
| Assignment | A durable handoff from one Work participant to another. It records intent and state but does not dispatch execution. |
| Decision | An independently queryable statement and optional rationale recorded for a Work. |
| Artifact | A durable reference from a Work to a repository-owned file, patch, commit, report, review, or note. |
| Handoff | A human-authorized transfer of an Assignment and bounded Work context to a Participant. |
| Participant adapter | The ephemeral boundary that prepares a provider-neutral Handoff for an installed external execution system. |
| FeltDB owner | The one live process owning a local database path. Clones and repeated opens share state inside it; another process is rejected. |
| Conversation reference | Provider-neutral identity for an external conversation. The provider retains the conversation and its history. |
| Work conversation | Durable relationship between a Work, Participant, and external Conversation reference. It is not a conversation mirror. |
| Contribution | A bounded Turn supplied locally or imported from an external conversation with explicit provenance. |
| Proposal | A provider-neutral statement being considered by a Work, with explicit author, provenance, and lifecycle. |
| Proposal review | A durable participant evaluation that approves, rejects, or requests changes to a Proposal. It is not itself authorization. |
| Coordination state | The deterministic current-state projection of proposals, reviews, decisions, assignments, and recent results. |
| Execution | One durable, provider-neutral materialization of an authorized Assignment, from start through one terminal outcome. |
| Execution provider | A replaceable adapter that performs an Assignment and reports observations to canonical Work transitions. |
| Execution result | The bounded canonical outcome of an Execution. Provider transcripts remain external. |

Out of vocabulary: agent session, workbench, desk, execution host, orcad, theme, setting, preference pane.
