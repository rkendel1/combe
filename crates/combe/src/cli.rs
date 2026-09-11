use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::chatgpt_adapter::{
    ChatGptAdapter, ConversationParticipantAdapter, ExternalContribution,
};
use crate::credential_store::{Credential, CredentialStore, KeychainCredentialStore};
use crate::participant_adapter::{
    ContextPackage, LocalCliAdapter, LocalProvider, ParticipantAdapter,
};
use crate::provider_router::{
    ChatGptManualAdapter, ChatGptProvider, ClaudeCodeAdapter, MessageRouter, OllamaAdapter,
    OpenAiAdapter, RoutingService,
};
use chrono::Utc;
use combe_catalog::{
    Catalog, State, add_repo, catalog, cleanup, load_state, remove_repo, save_state, state_path,
};
use combe_state::{
    AiContextEventData, AiContinuityStore, AiProviderExchange, ArtifactId, ArtifactKind,
    AssignmentId, AssignmentStatus, ChatGptHistoryStore, ContextAssembler, ContextGraph,
    ContextGraphProjector, ContextGraphStore, ContextRequest, ContributionAcceptance,
    ContributionKind, ConversationId, ConversationProvider, ConversationRef, CredentialRef,
    ExecutionId, ExecutionMode, ExecutionStatus, FeltDbWorkStore, FeltDbWorkspaceStore,
    GraphQueryOptions, Participant, ParticipantKind, ParticipantResult, ProposalId, ProposalReview,
    ProposalStatus, ProviderCapabilities, ProviderProfile, ProviderProfileId, ProviderRegistry,
    ProviderService, Recipient, RecipientId, ReviewId, ReviewOutcome, TurnId, TurnKind, TurnOrigin,
    WORK_CONTEXT_VERSION, Work, WorkAction, WorkArtifact, WorkAssignment, WorkContribution,
    WorkConversation, WorkDecision, WorkExecution, WorkId, WorkProposal, WorkStore, WorkTurn,
    WorkspaceStore, derive_work_attention, derive_work_attention_with_health,
};

const USAGE: &str = "\
combe — a worktree-aware terminal

Usage:
  combe list                Show registered repos and their worktrees
  combe add <path>...       Register repos
  combe remove <path>...    Unregister repos
  combe cleanup             Drop registered paths that no longer exist on disk
  combe open <path>         Open or focus a workspace
  combe doctor              Check Combe installation and state
  combe chatgpt import <export.zip> [--dry-run] [--json] [--search <text>] [--from <date>] [--to <date>] [--conversation <id>]
  combe work list [--workspace <id>]
  combe work create <title> [--workspace <id>] [--objective <text>]
  combe work show <id>
  combe work activity <id> [--json]
  combe work exchange <exchange-id> [--json]
  combe work result <exchange-id> [--json]
  combe work next <work-id> [--json]
  combe work attention <work-id> [--json]
  combe work attention --all [--json]
  combe work context <id> [--format <text|json>]
  combe work protocol <id> [--participant <name-or-id>] [--action <action>] [--format <text|json>]
  combe work chatgpt <id> [--action <inspect|propose|review>] [--conversation <id>] [--title <title>]
  combe work chatgpt import <id> --kind <message|proposal|review> [--revision <revision>] [--execution <id>]
  combe work review <id> --participant <name-or-id> --execution <id> [--revision <revision>]
  combe work decision <id> <statement> [--rationale <text>]
  combe work proposal create <work-id> --participant <name-or-id> --title <title>
  combe work proposal list <work-id>
  combe work proposal show <proposal-id>
  combe work proposal approve <proposal-id> --by <participant>
  combe work proposal reject <proposal-id> --by <participant>
  combe work proposal request-changes <proposal-id> --by <participant>
  combe work assign <id> --to <participant> [--proposal <id>] --instruction <text>
  combe work handoff <id> --to <participant> --provider <codex|claude>
  combe work start <assignment-id> --provider <name> [--provider-id <id>]
  combe work status <assignment-id>
  combe work heartbeat <assignment-id>
  combe work complete <assignment-id>
  combe work fail <assignment-id> [--failure <text>]
  combe work cancel <assignment-id> [--reason <text>]
  combe work turn <id> --participant <name-or-id> [--kind <kind>]
  combe work contribute <id> --participant <name-or-id> [--kind <kind>]
  combe work import <id> --participant <name-or-id> --conversation <id> [--provider <provider>] [--title <title>] [--kind <kind>]
  combe work status <work-id> <active|paused|completed|archived>
  combe provider <list|show|create|enable|disable|delete|test|credential-set|credential-delete> ...
  combe recipient <list|show|create|enable|disable|delete> ...
  combe route send <work-id> --to <recipient-id>
  combe graph status [--json]
  combe graph rebuild [--dry-run] [--verify] [--work <id>] [--worktree <id>] [--repository <id>] [--json]
  combe graph verify [--json]
  combe graph inspect <node-id> [--json]
  combe graph related <node-id> [--json]
  combe context <preview|inspect|fingerprint> <work-id> --recipient <id> [--message <text>] [--max-items <n>] [--max-bytes <n>] [--json]
  combe context diff <work-id> <fingerprint> --recipient <id> [--message <text>] [--json]
  combe version             Show version information
  combe --fresh             Start without restoring previous session
  combe help                Show this help

The window opens from the Dock, Finder, or `open -a Combe`.
";

pub fn run() -> Option<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        if launched_from_bundle() {
            return None;
        }
        print_usage();
        return Some(ExitCode::SUCCESS);
    };
    let rest = &args[1..];
    match command {
        "list" => Some(list()),
        "add" => Some(add(rest)),
        "remove" => Some(remove(rest)),
        "cleanup" => Some(clean()),
        "open" => Some(open_workspace(rest)),
        "doctor" => Some(doctor()),
        "chatgpt" => Some(chatgpt_history(rest)),
        "work" => Some(work(rest)),
        "provider" => Some(provider(rest)),
        "recipient" => Some(recipient(rest)),
        "route" => Some(route(rest)),
        "graph" => Some(graph(rest)),
        "context" => Some(context_command(rest)),
        "version" | "-v" | "--version" => Some(version()),
        "--fresh" => None,
        "help" | "-h" | "--help" => {
            print_usage();
            Some(ExitCode::SUCCESS)
        }
        other => {
            eprintln!("combe: unknown command '{other}'");
            print_usage();
            Some(ExitCode::from(2))
        }
    }
}

fn context_command(args: &[String]) -> ExitCode {
    let Some(command @ ("preview" | "inspect" | "fingerprint" | "diff")) =
        args.first().map(String::as_str)
    else {
        eprintln!("combe: context needs preview, inspect, fingerprint, or diff");
        return ExitCode::from(2);
    };
    let Some(work_id) = args.get(1).map(|value| WorkId(value.clone())) else {
        eprintln!("combe: context {command} needs a Work id");
        return ExitCode::from(2);
    };
    let Some(recipient_id) = option(args, "--recipient").map(RecipientId) else {
        eprintln!("combe: context {command} needs --recipient <id>");
        return ExitCode::from(2);
    };
    let message = option(args, "--message").unwrap_or_else(|| "Continue this Work.".into());
    let mut request = ContextRequest::new(work_id, recipient_id, message);
    if let Some(value) = option(args, "--max-items") {
        request.max_items = match value.parse() {
            Ok(value) => value,
            Err(_) => return cli_value_error("--max-items must be a number"),
        };
    }
    if let Some(value) = option(args, "--max-bytes") {
        request.max_bytes = match value.parse() {
            Ok(value) => value,
            Err(_) => return cli_value_error("--max-bytes must be a number"),
        };
    }
    if let Some(value) = option(args, "--max-reference-bytes") {
        request.max_reference_bytes = match value.parse() {
            Ok(value) => value,
            Err(_) => return cli_value_error("--max-reference-bytes must be a number"),
        };
    }
    if let Some(value) = option(args, "--max-content-bytes") {
        request.max_content_bytes = match value.parse() {
            Ok(value) => value,
            Err(_) => return cli_value_error("--max-content-bytes must be a number"),
        };
    }
    if let Some(value) = option(args, "--max-file-bytes") {
        request.max_file_bytes = match value.parse() {
            Ok(value) => value,
            Err(_) => return cli_value_error("--max-file-bytes must be a number"),
        };
    }
    let store = match FeltDbWorkStore::for_combe() {
        Ok(store) => store,
        Err(error) => return work_error(error),
    };
    let package = match ContextAssembler::assemble(&store, &request) {
        Ok(package) => package,
        Err(error) => return work_error(error),
    };
    let json = args.iter().any(|value| value == "--json");
    if command == "fingerprint" {
        println!("{}", package.fingerprint);
        return ExitCode::SUCCESS;
    }
    if command == "diff" {
        let Some(previous) = args.get(2).filter(|value| !value.starts_with("--")) else {
            return cli_value_error("context diff needs a previous fingerprint");
        };
        let previous_items = store
            .ai_conversation_ids()
            .unwrap_or_default()
            .into_iter()
            .flat_map(|conversation| store.ai_exchanges(&conversation).unwrap_or_default())
            .find(|exchange| exchange.context_fingerprint == *previous)
            .map(|exchange| exchange.context_items)
            .unwrap_or_default();
        let current_items = package
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let added = current_items
            .iter()
            .filter(|item| !previous_items.contains(item))
            .collect::<Vec<_>>();
        let removed = previous_items
            .iter()
            .filter(|item| !current_items.contains(item))
            .collect::<Vec<_>>();
        let value = serde_json::json!({
            "previous_fingerprint": previous,
            "current_fingerprint": package.fingerprint,
            "changed": previous != &package.fingerprint,
            "added": added,
            "removed": removed,
            "current_items": current_items,
            "previous_items": previous_items
        });
        return match graph_print(&value, json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => work_error(error),
        };
    }
    if json || command == "inspect" {
        return match graph_print(&package, json) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => work_error(error),
        };
    }
    println!(
        "{} items · {} bytes{}\nworktree: {}\nprovider: {}{}\nfingerprint: {}",
        package.items.len(),
        package.bytes,
        if package.truncated { " · bounded" } else { "" },
        package.worktree,
        package.provider,
        package
            .model
            .as_deref()
            .map(|model| format!(" · {model}"))
            .unwrap_or_default(),
        package.fingerprint
    );
    for item in package.items {
        println!("\n{:?} · {}\n{}", item.kind, item.display, item.reference);
        println!(
            "  content: {:?}{}",
            item.content_status,
            item.byte_size
                .map(|bytes| format!(" · {bytes} bytes"))
                .unwrap_or_default()
        );
        for reason in item.reasons {
            println!("  - {reason}");
        }
        if let Some(content) = item.content {
            println!("\n--- exact provider content ---\n{content}\n--- end content ---");
        }
    }
    ExitCode::SUCCESS
}

fn cli_value_error(message: &str) -> ExitCode {
    eprintln!("combe: {message}");
    ExitCode::from(2)
}

fn graph(args: &[String]) -> ExitCode {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("combe: graph needs status, rebuild, verify, inspect, or related");
        return ExitCode::from(2);
    };
    let store = match FeltDbWorkStore::for_combe() {
        Ok(store) => store,
        Err(error) => return work_error(error),
    };
    let json = args.iter().any(|value| value == "--json");
    let result = match command {
        "status" | "verify" => {
            ContextGraphProjector::verify(&store).and_then(|health| graph_print(&health, json))
        }
        "rebuild" => graph_rebuild(&store, &args[1..], json),
        "inspect" | "related" => graph_inspect(&store, &args[1..], command == "related", json),
        other => {
            eprintln!("combe: unknown graph command '{other}'");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn graph_rebuild(store: &FeltDbWorkStore, args: &[String], json: bool) -> combe_state::Result<()> {
    let dry_run = args.iter().any(|value| value == "--dry-run");
    let verify = args.iter().any(|value| value == "--verify");
    let projection = ContextGraphProjector::build(store)?;
    let scope = option(args, "--work")
        .map(|id| format!("work:{id}"))
        .or_else(|| option(args, "--worktree").map(|id| format!("worktree:{id}")));
    if let Some(repository) = option(args, "--repository") {
        let nodes = projection
            .nodes
            .iter()
            .filter(|node| node.properties.get("repository_id") == Some(&repository))
            .count();
        return graph_print(
            &serde_json::json!({
                "dry_run": true,
                "scope": { "repository": repository },
                "nodes": nodes,
                "graph_fingerprint": projection.graph_fingerprint
            }),
            json,
        );
    }
    if let Some(root) = scope {
        let subgraph = projection.traverse(&root, &GraphQueryOptions::default())?;
        return graph_print(&subgraph, json);
    }
    if verify {
        return graph_print(&ContextGraphProjector::verify(store)?, json);
    }
    if !dry_run {
        store.save_context_graph(&projection)?;
    }
    graph_print(
        &serde_json::json!({
            "dry_run": dry_run,
            "contract": projection.contract,
            "version": projection.version,
            "graph_fingerprint": projection.graph_fingerprint,
            "source_revision": projection.source_revision,
            "nodes": projection.nodes.len(),
            "edges": projection.edges.len()
        }),
        json,
    )
}

fn graph_inspect(
    store: &FeltDbWorkStore,
    args: &[String],
    related: bool,
    json: bool,
) -> combe_state::Result<()> {
    let requested = args.first().ok_or_else(|| {
        combe_state::StateError::InvalidEntity("graph inspect needs a node identity".into())
    })?;
    let projection =
        store
            .load_context_graph()?
            .ok_or_else(|| combe_state::StateError::NotFound {
                entity_type: "context-graph".into(),
                id: "v1".into(),
            })?;
    let id = projection
        .nodes
        .iter()
        .find(|node| &node.id == requested)
        .or_else(|| {
            requested.strip_prefix("file:").and_then(|path| {
                projection.nodes.iter().find(|node| {
                    node.properties
                        .get("path")
                        .is_some_and(|value| value == path)
                })
            })
        })
        .map(|node| node.id.as_str())
        .ok_or_else(|| combe_state::StateError::NotFound {
            entity_type: "context-node".into(),
            id: requested.clone(),
        })?;
    if related {
        graph_print(
            &projection.related(id, &GraphQueryOptions::default())?,
            json,
        )
    } else {
        let node = projection.nodes.iter().find(|node| node.id == id).unwrap();
        graph_print(node, json)
    }
}

fn graph_print(value: &impl serde::Serialize, json: bool) -> combe_state::Result<()> {
    if json {
        println!("{}", serde_json::to_string(value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

fn chatgpt_history(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) != Some("import") {
        eprintln!("combe: chatgpt needs 'import <export.zip>'");
        return ExitCode::from(2);
    }
    let Some(path) = args.get(1) else {
        eprintln!("combe: chatgpt import needs an export ZIP or conversation JSON file");
        return ExitCode::from(2);
    };
    let acquisition = match crate::chatgpt_history::acquire(Path::new(path)) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("combe: {error}");
            return ExitCode::FAILURE;
        }
    };
    let parse_date = |name: &str| {
        option(args, name)
            .map(|value| chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d"))
            .transpose()
    };
    let from = match parse_date("--from") {
        Ok(value) => value,
        Err(_) => {
            eprintln!("combe: --from must use YYYY-MM-DD");
            return ExitCode::from(2);
        }
    };
    let through = match parse_date("--to") {
        Ok(value) => value,
        Err(_) => {
            eprintln!("combe: --to must use YYYY-MM-DD");
            return ExitCode::from(2);
        }
    };
    let search = option(args, "--search").unwrap_or_default();
    let requested_id = option(args, "--conversation");
    let selected = acquisition
        .conversations
        .iter()
        .filter(|conversation| {
            let date = conversation
                .updated_at
                .or(conversation.created_at)
                .map(|value| value.date_naive());
            crate::chatgpt_history::matches(conversation, &search)
                && requested_id
                    .as_ref()
                    .is_none_or(|id| &conversation.conversation_id == id)
                && from.is_none_or(|from| date.is_some_and(|date| date >= from))
                && through.is_none_or(|through| date.is_some_and(|date| date <= through))
        })
        .cloned()
        .collect::<Vec<_>>();
    let message_count = selected
        .iter()
        .map(|conversation| conversation.messages.len())
        .sum::<usize>();
    let dry_run = args.iter().any(|value| value == "--dry-run");
    let json_output = args.iter().any(|value| value == "--json");
    if dry_run {
        if json_output {
            println!(
                "{}",
                serde_json::json!({
                    "dry_run": true,
                    "source": acquisition.source.display_name,
                    "conversations_found": acquisition.conversations.len(),
                    "conversations_selected": selected.len(),
                    "messages_selected": message_count,
                    "empty_conversations": acquisition.empty_count,
                    "conversations_with_attachments": acquisition.attachment_count
                })
            );
        } else {
            println!("ChatGPT export: {}", acquisition.source.display_name);
            println!("Conversations found: {}", acquisition.conversations.len());
            println!("Conversations selected: {}", selected.len());
            println!("Messages selected: {message_count}");
            println!("Dry run: no state changed");
        }
        return ExitCode::SUCCESS;
    }
    if selected.is_empty() {
        eprintln!("combe: no conversations match the selection");
        return ExitCode::from(2);
    }
    let store = match FeltDbWorkStore::for_combe() {
        Ok(store) => store,
        Err(error) => return work_error(error),
    };
    match store.import_chatgpt_source(acquisition.source, selected) {
        Ok(result) => {
            if json_output {
                println!("{}", serde_json::to_string(&result).unwrap());
            } else {
                println!("New conversations: {}", result.new_conversations);
                println!("Updated conversations: {}", result.updated_conversations);
                println!("New messages: {}", result.new_messages);
                println!("Duplicates: {}", result.duplicate_conversations);
                println!("Skipped: {}", result.skipped_conversations);
            }
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work(args: &[String]) -> ExitCode {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("combe: work needs a command");
        return ExitCode::from(2);
    };
    let store = match FeltDbWorkStore::for_combe() {
        Ok(store) => store,
        Err(error) => return work_error(error),
    };
    match command {
        "list" => work_list(&store, &args[1..]),
        "create" => work_create(&store, &args[1..]),
        "show" => work_show(&store, &args[1..]),
        "activity" => work_activity(&store, &args[1..]),
        "exchange" => work_exchange(&store, &args[1..], false),
        "result" => work_exchange(&store, &args[1..], true),
        "next" => work_attention(&store, &args[1..], true),
        "attention" => work_attention(&store, &args[1..], false),
        "context" => work_context(&store, &args[1..]),
        "protocol" => work_protocol(&store, &args[1..]),
        "chatgpt" => work_chatgpt(&store, &args[1..]),
        "review" => work_review(&store, &args[1..]),
        "decision" => work_decision(&store, &args[1..]),
        "proposal" => work_proposal(&store, &args[1..]),
        "assign" => work_assign(&store, &args[1..]),
        "handoff" => work_handoff(&store, &args[1..]),
        "start" => work_start(&store, &args[1..]),
        "heartbeat" => work_heartbeat(&store, &args[1..]),
        "complete" => work_finish_execution(&store, &args[1..], ExecutionStatus::Completed),
        "fail" => work_finish_execution(&store, &args[1..], ExecutionStatus::Failed),
        "cancel" => work_finish_execution(&store, &args[1..], ExecutionStatus::Cancelled),
        "turn" => work_turn(&store, &args[1..]),
        "contribute" => work_contribute(&store, &args[1..]),
        "import" => work_import(&store, &args[1..]),
        "status" if args.len() == 2 => work_execution_status(&store, &args[1..]),
        "status" => work_status(&store, &args[1..]),
        other => {
            eprintln!("combe: unknown work command '{other}'");
            ExitCode::from(2)
        }
    }
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn provider(args: &[String]) -> ExitCode {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("combe: provider needs a command");
        return ExitCode::from(2);
    };
    let store = match FeltDbWorkStore::for_combe() {
        Ok(store) => store,
        Err(error) => return work_error(error),
    };
    let result = match command {
        "list" => store.list_profiles().map(|profiles| {
            for profile in profiles {
                println!(
                    "{}\t{}\t{:?}\t{}\t{:?}\t{}",
                    profile.id,
                    profile.name,
                    profile.service,
                    profile.model.as_deref().unwrap_or("-"),
                    profile.execution_mode,
                    if profile.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
            }
        }),
        "show" => args
            .get(1)
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("provider show needs an id".into())
            })
            .and_then(|id| store.get_profile(&ProviderProfileId(id.clone())))
            .map(|profile| {
                println!("id: {}", profile.id);
                println!("name: {}", profile.name);
                println!("service: {:?}", profile.service);
                println!("model: {}", profile.model.as_deref().unwrap_or("-"));
                println!("execution: {:?}", profile.execution_mode);
                println!("endpoint: {}", profile.endpoint.as_deref().unwrap_or("-"));
                println!(
                    "credential: {}",
                    profile
                        .credential_ref
                        .as_ref()
                        .map(|value| value.0.as_str())
                        .unwrap_or("-")
                );
            }),
        "create" => create_provider(&store, &args[1..]),
        "enable" | "disable" => set_provider_enabled(&store, &args[1..], command == "enable"),
        "delete" => args
            .get(1)
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("provider delete needs an id".into())
            })
            .and_then(|id| store.delete_profile(&ProviderProfileId(id.clone()))),
        "test" => test_provider(&store, &args[1..]),
        "credential-set" => set_provider_credential(&args[1..]),
        "credential-delete" => delete_provider_credential(&args[1..]),
        other => Err(combe_state::StateError::InvalidEntity(format!(
            "unknown provider command {other}"
        ))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn set_provider_enabled(
    store: &FeltDbWorkStore,
    args: &[String],
    enabled: bool,
) -> combe_state::Result<()> {
    let id = args.first().ok_or_else(|| {
        combe_state::StateError::InvalidEntity("provider enable/disable needs an id".into())
    })?;
    let mut profile = store.get_profile(&ProviderProfileId(id.clone()))?;
    profile.enabled = enabled;
    store.update_profile(profile)
}

fn create_provider(store: &FeltDbWorkStore, args: &[String]) -> combe_state::Result<()> {
    let name = option(args, "--name").ok_or_else(|| {
        combe_state::StateError::InvalidEntity("provider create needs --name".into())
    })?;
    let service = parse_service(option(args, "--service").as_deref())?;
    let execution_mode = parse_execution_mode(option(args, "--mode").as_deref())?;
    let profile = ProviderProfile {
        id: ProviderProfileId::new(),
        name,
        service,
        model: option(args, "--model"),
        capabilities: ProviderCapabilities {
            text_generation: true,
            code_execution: args.iter().any(|value| value == "--code-execution"),
            structured_output: args.iter().any(|value| value == "--structured-output"),
        },
        execution_mode,
        credential_ref: option(args, "--credential-ref").map(CredentialRef),
        endpoint: option(args, "--endpoint"),
        enabled: true,
    };
    store.create_profile(profile.clone())?;
    println!("{}", profile.id);
    Ok(())
}

fn parse_service(value: Option<&str>) -> combe_state::Result<ProviderService> {
    match value {
        Some("ollama") => Ok(ProviderService::Ollama),
        Some("claude_code") => Ok(ProviderService::ClaudeCode),
        Some("chatgpt") => Ok(ProviderService::ChatGpt),
        Some("openai_api") => Ok(ProviderService::OpenAiApi),
        _ => Err(combe_state::StateError::InvalidEntity(
            "service must be ollama, claude_code, chatgpt, or openai_api".into(),
        )),
    }
}

fn parse_execution_mode(value: Option<&str>) -> combe_state::Result<ExecutionMode> {
    match value {
        Some("local_http") => Ok(ExecutionMode::LocalHttp),
        Some("local_cli") => Ok(ExecutionMode::LocalCli),
        Some("external_manual") => Ok(ExecutionMode::ExternalManual),
        Some("http_api") => Ok(ExecutionMode::HttpApi),
        _ => Err(combe_state::StateError::InvalidEntity(
            "mode must be local_http, local_cli, external_manual, or http_api".into(),
        )),
    }
}

fn adapters() -> (
    OllamaAdapter,
    ClaudeCodeAdapter,
    ChatGptManualAdapter,
    ChatGptProvider,
    OpenAiAdapter,
) {
    (
        OllamaAdapter::new(),
        ClaudeCodeAdapter,
        ChatGptManualAdapter,
        ChatGptProvider::new(),
        OpenAiAdapter::new(),
    )
}

fn test_provider(store: &FeltDbWorkStore, args: &[String]) -> combe_state::Result<()> {
    let id = args.first().ok_or_else(|| {
        combe_state::StateError::InvalidEntity("provider test needs an id".into())
    })?;
    let profile = store.get_profile(&ProviderProfileId(id.clone()))?;
    let credentials = KeychainCredentialStore::new();
    let (ollama, claude, chatgpt, chatgpt_api, openai) = adapters();
    let router = RoutingService::new(
        store,
        &credentials,
        vec![&ollama, &claude, &chatgpt, &chatgpt_api, &openai],
    );
    let status = router
        .test_profile(&profile)
        .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
    println!("{status}");
    Ok(())
}

fn set_provider_credential(args: &[String]) -> combe_state::Result<()> {
    let reference = args.first().ok_or_else(|| {
        combe_state::StateError::InvalidEntity("credential-set needs a reference".into())
    })?;
    let value = stdin_content()
        .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
    if value.trim().is_empty() {
        return Err(combe_state::StateError::InvalidEntity(
            "credential cannot be empty".into(),
        ));
    }
    KeychainCredentialStore::new()
        .set(
            &CredentialRef(reference.clone()),
            Credential::new(value.trim().as_bytes()),
        )
        .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))
}

fn delete_provider_credential(args: &[String]) -> combe_state::Result<()> {
    let reference = args.first().ok_or_else(|| {
        combe_state::StateError::InvalidEntity("credential-delete needs a reference".into())
    })?;
    KeychainCredentialStore::new()
        .delete(&CredentialRef(reference.clone()))
        .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))
}

fn recipient(args: &[String]) -> ExitCode {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("combe: recipient needs a command");
        return ExitCode::from(2);
    };
    let store = match FeltDbWorkStore::for_combe() {
        Ok(store) => store,
        Err(error) => return work_error(error),
    };
    let result = match command {
        "list" => store.list_recipients().map(|recipients| {
            for recipient in recipients {
                println!(
                    "{}\t{}\t{}\t{}",
                    recipient.id,
                    recipient.name,
                    recipient.provider_profile_id,
                    if recipient.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
            }
        }),
        "show" => args
            .get(1)
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("recipient show needs an id".into())
            })
            .and_then(|id| store.get_recipient(&RecipientId(id.clone())))
            .map(|recipient| {
                println!("id: {}", recipient.id);
                println!("name: {}", recipient.name);
                println!("provider_profile: {}", recipient.provider_profile_id);
                println!("enabled: {}", recipient.enabled);
            }),
        "create" => {
            let name = option(args, "--name").ok_or_else(|| {
                combe_state::StateError::InvalidEntity("recipient create needs --name".into())
            });
            let profile = option(args, "--provider").ok_or_else(|| {
                combe_state::StateError::InvalidEntity("recipient create needs --provider".into())
            });
            name.and_then(|name| profile.map(|profile| (name, profile)))
                .and_then(|(name, profile)| {
                    let recipient = Recipient {
                        id: RecipientId::new(),
                        name,
                        provider_profile_id: ProviderProfileId(profile),
                        enabled: true,
                    };
                    store.create_recipient(recipient.clone())?;
                    println!("{}", recipient.id);
                    Ok(())
                })
        }
        "delete" => args
            .get(1)
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("recipient delete needs an id".into())
            })
            .and_then(|id| store.delete_recipient(&RecipientId(id.clone()))),
        "enable" | "disable" => args
            .get(1)
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity(
                    "recipient enable/disable needs an id".into(),
                )
            })
            .and_then(|id| store.get_recipient(&RecipientId(id.clone())))
            .and_then(|mut recipient| {
                recipient.enabled = command == "enable";
                store.update_recipient(recipient)
            }),
        other => Err(combe_state::StateError::InvalidEntity(format!(
            "unknown recipient command {other}"
        ))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn route(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) != Some("send") {
        eprintln!("combe: route needs send");
        return ExitCode::from(2);
    }
    let Some(work_id) = args.get(1).map(|value| WorkId(value.clone())) else {
        eprintln!("combe: route send needs a Work id");
        return ExitCode::from(2);
    };
    let Some(recipient_id) = option(args, "--to").map(RecipientId) else {
        eprintln!("combe: route send needs --to");
        return ExitCode::from(2);
    };
    let message = match stdin_content() {
        Ok(message) => message,
        Err(error) => {
            eprintln!("combe: {error}");
            return ExitCode::from(1);
        }
    };
    let store = match FeltDbWorkStore::for_combe() {
        Ok(store) => store,
        Err(error) => return work_error(error),
    };
    let credentials = KeychainCredentialStore::new();
    let (ollama, claude, chatgpt, chatgpt_api, openai) = adapters();
    let router = RoutingService::new(
        &store,
        &credentials,
        vec![&ollama, &claude, &chatgpt, &chatgpt_api, &openai],
    );
    match router.send(&work_id, &recipient_id, &message) {
        Ok(result) => {
            print!("{}", result.content);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("combe: {error}");
            ExitCode::from(1)
        }
    }
}

fn bounded_execution_output(value: String) -> String {
    let limit = combe_state::work_store::CONTEXT_TEXT_LIMIT;
    if value.len() <= limit {
        return value;
    }
    let suffix = "\n[provider output truncated]";
    let mut end = limit.saturating_sub(suffix.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}", &value[..end])
}

fn work_list(store: &impl WorkStore, args: &[String]) -> ExitCode {
    match store.list_works(option(args, "--workspace").as_deref()) {
        Ok(works) => {
            for work in works {
                println!(
                    "{}\t{:?}\t{}\t{}",
                    work.id, work.status, work.workspace_id, work.title
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn current_workspace() -> Result<String, std::io::Error> {
    Ok(std::fs::canonicalize(std::env::current_dir()?)?
        .to_string_lossy()
        .into_owned())
}

fn work_create(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(title) = args.first().filter(|value| !value.starts_with("--")) else {
        eprintln!("combe: work create needs a title");
        return ExitCode::from(2);
    };
    let workspace = option(args, "--workspace").or_else(|| current_workspace().ok());
    let Some(workspace) = workspace else {
        eprintln!("combe: cannot determine the current workspace");
        return ExitCode::FAILURE;
    };
    let work = Work::new(workspace, title.clone(), option(args, "--objective"));
    let human = Participant::new(work.id.clone(), ParticipantKind::Human, "Human".into());
    match store.create_work(work.clone(), vec![human]) {
        Ok(()) => {
            println!("{}", work.id);
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn parse_work_id(args: &[String]) -> Option<WorkId> {
    args.first().map(|id| WorkId(id.clone()))
}

fn work_show(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work show needs an id");
        return ExitCode::from(2);
    };
    match store.context(&id) {
        Ok(context) => {
            println!("{} [{:?}]", context.work.title, context.work.status);
            println!("id: {}", context.work.id);
            println!("workspace: {}", context.work.workspace_id);
            if let Some(objective) = &context.work.objective {
                println!("objective: {objective}");
            }
            println!("participants: {}", context.participants.len());
            println!("turns: {}", context.recent_turns.len());
            println!("active assignments: {}", context.active_assignments.len());
            println!("decisions: {}", context.decisions.len());
            println!("artifacts: {}", context.artifacts.len());
            println!("conversations: {}", context.conversations.len());
            println!("active proposals: {}", context.state.active_proposals.len());
            println!(
                "approved proposals: {}",
                context.state.approved_proposals.len()
            );
            println!("recent results: {}", context.state.recent_results.len());
            println!("executions: {}", context.executions.len());
            println!("next: {}", crate::work_overview::next_action(&context));
            for execution in &context.executions {
                println!(
                    "execution {}: {:?} · assignment {} · provider {} · result {}",
                    execution.id,
                    execution.status,
                    execution.assignment_id,
                    execution.provider,
                    execution.result_id.as_deref().unwrap_or("none")
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_activity(store: &FeltDbWorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work activity needs a Work id");
        return ExitCode::from(2);
    };
    let context = match store.context(&id) {
        Ok(context) => context,
        Err(error) => return work_error(error),
    };
    let exchanges = match store.ai_exchanges(&format!("work:{}", id.0)) {
        Ok(exchanges) => exchanges,
        Err(error) => return work_error(error),
    };
    let mut entries = Vec::new();
    for proposal in &context.proposals {
        entries.push((
            proposal.created_at,
            serde_json::json!({
                "at": proposal.created_at,
                "kind": "proposal.created",
                "actor": participant_label(&context, &proposal.proposed_by.0),
                "summary": proposal.title,
                "proposal_id": proposal.id
            }),
        ));
    }
    for review in &context.reviews {
        entries.push((
            review.created_at,
            serde_json::json!({
                "at": review.created_at,
                "kind": "proposal.reviewed",
                "actor": participant_label(&context, &review.reviewed_by.0),
                "summary": format!("{:?}", review.outcome),
                "proposal_id": review.proposal_id
            }),
        ));
    }
    for assignment in &context.assignments {
        entries.push((
            assignment.created_at,
            serde_json::json!({
                "at": assignment.created_at,
                "kind": "assignment.created",
                "actor": participant_label(&context, &assignment.to_participant_id.0),
                "summary": assignment.instruction,
                "assignment_id": assignment.id
            }),
        ));
    }
    for execution in &context.executions {
        entries.push((
            execution.started_at,
            serde_json::json!({
                "at": execution.started_at,
                "kind": "execution.started",
                "actor": execution.provider,
                "summary": format!("Execution {:?}", execution.status),
                "execution_id": execution.id
            }),
        ));
    }
    for exchange in exchanges {
        entries.push((exchange.created_at, serde_json::json!({
            "at": exchange.created_at,
            "kind": "provider.exchange",
            "actor": exchange.provider,
            "summary": format!("{:?} · {} context items", exchange.status, exchange.context_items.len()),
            "exchange_id": exchange.id,
            "recipient_id": exchange.recipient_id,
            "provider_profile_id": exchange.provider_profile_id,
            "context_fingerprint": exchange.context_fingerprint
        })));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let values = entries
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--json") {
        return match serde_json::to_string_pretty(&values) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => work_error(error),
        };
    }
    println!("{}\n", context.work.title);
    for value in values {
        println!(
            "{}  {}\n  {}\n  {}",
            value["at"].as_str().unwrap_or(""),
            value["actor"].as_str().unwrap_or("Combe"),
            value["summary"].as_str().unwrap_or(""),
            value["kind"].as_str().unwrap_or("")
        );
        if let Some(exchange) = value["exchange_id"].as_str() {
            println!("  exchange: {exchange}");
        }
        if let Some(fingerprint) = value["context_fingerprint"].as_str() {
            println!("  context: {fingerprint}");
        }
        println!();
    }
    ExitCode::SUCCESS
}

fn work_attention(store: &FeltDbWorkStore, args: &[String], next_only: bool) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    if args.iter().any(|arg| arg == "--all") {
        if next_only {
            return cli_value_error("work next needs one Work id");
        }
        let health = match ContextGraphProjector::verify(store) {
            Ok(health) => health,
            Err(error) => return work_error(error),
        };
        let mut works = match store.list_works(None) {
            Ok(works) => works,
            Err(error) => return work_error(error),
        };
        works.sort_by(|left, right| left.id.0.cmp(&right.id.0));
        let mut projections = Vec::new();
        for work in works {
            match derive_work_attention_with_health(store, &work.id, &health) {
                Ok(attention) => projections.push(attention),
                Err(error) => return work_error(error),
            }
        }
        if json {
            return match serde_json::to_string_pretty(&projections) {
                Ok(value) => {
                    println!("{value}");
                    ExitCode::SUCCESS
                }
                Err(error) => work_error(error),
            };
        }
        for attention in projections {
            println!("{}  {} actions", attention.work_id, attention.items.len());
            if let Some(item) = attention.items.first() {
                println!("  {:?}: {}", item.priority, item.title);
                println!("  {}", item.reason);
            }
        }
        return ExitCode::SUCCESS;
    }
    let Some(id) = args.iter().find(|arg| !arg.starts_with("--")) else {
        eprintln!(
            "combe: work {} needs a Work id",
            if next_only { "next" } else { "attention" }
        );
        return ExitCode::from(2);
    };
    let attention = match derive_work_attention(store, &WorkId(id.clone())) {
        Ok(attention) => attention,
        Err(error) => return work_error(error),
    };
    if json {
        if next_only {
            let value = serde_json::json!({
                "work_id": attention.work_id,
                "revision": attention.revision,
                "item": attention.items.first()
            });
            return match serde_json::to_string_pretty(&value) {
                Ok(value) => {
                    println!("{value}");
                    ExitCode::SUCCESS
                }
                Err(error) => work_error(error),
            };
        }
        return match serde_json::to_string_pretty(&attention) {
            Ok(value) => {
                println!("{value}");
                ExitCode::SUCCESS
            }
            Err(error) => work_error(error),
        };
    }
    if next_only {
        if let Some(item) = attention.items.first() {
            println!("{:?}  {}", item.priority, item.title);
            println!("{}", item.reason);
            if let Some(id) = &item.related_id {
                println!("related: {id}");
            }
        } else {
            println!("Nothing requires attention.");
        }
        return ExitCode::SUCCESS;
    }
    println!(
        "{} · revision {} · {} actions",
        attention.work_id,
        attention.revision,
        attention.items.len()
    );
    for item in attention.items {
        println!("\n{:?}  {}", item.priority, item.title);
        println!("{}", item.reason);
        if let Some(id) = item.related_id {
            println!("related: {id}");
        }
    }
    ExitCode::SUCCESS
}

fn participant_label(context: &combe_state::WorkContext, id: &str) -> String {
    context
        .participants
        .iter()
        .find(|participant| participant.id.0 == id)
        .map(|participant| participant.name.clone())
        .unwrap_or_else(|| id.into())
}

fn work_exchange(store: &FeltDbWorkStore, args: &[String], result_only: bool) -> ExitCode {
    let Some(id) = args.first() else {
        eprintln!(
            "combe: work {} needs an exchange id",
            if result_only { "result" } else { "exchange" }
        );
        return ExitCode::from(2);
    };
    let exchange = match find_exchange(store, id) {
        Ok(exchange) => exchange,
        Err(error) => return work_error(error),
    };
    let events = match store.ai_events(&exchange.conversation_id) {
        Ok(events) => events,
        Err(error) => return work_error(error),
    };
    let request = ai_message_content(&events, &exchange.input_message_id).unwrap_or_default();
    let response = ai_message_content(&events, &exchange.output_message_id).unwrap_or_default();
    let recipient = exchange
        .recipient_id
        .as_ref()
        .and_then(|id| store.get_recipient(id).ok())
        .map(|recipient| recipient.name);
    let profile = exchange
        .provider_profile_id
        .as_ref()
        .and_then(|id| store.get_profile(id).ok())
        .map(|profile| profile.name);
    let artifacts = exchange
        .work_id
        .as_ref()
        .and_then(|id| store.context(id).ok())
        .map(|context| context.artifacts)
        .unwrap_or_default();
    let value = serde_json::json!({
        "id": exchange.id,
        "status": exchange.status,
        "work_id": exchange.work_id,
        "conversation_id": exchange.conversation_id,
        "recipient": recipient,
        "recipient_id": exchange.recipient_id,
        "provider": exchange.provider,
        "provider_profile": profile,
        "provider_profile_id": exchange.provider_profile_id,
        "service": exchange.service,
        "model": exchange.model,
        "execution_mode": exchange.execution_mode,
        "context_fingerprint": exchange.context_fingerprint,
        "context_items": exchange.context_items,
        "context_item_count": exchange.context_items.len(),
        "context_bytes": exchange.context_bytes,
        "request": request,
        "response": response,
        "request_id": exchange.request_id,
        "usage": exchange.usage,
        "duration_ms": exchange.duration_ms,
        "error": exchange.error,
        "artifacts": artifacts,
        "created_at": exchange.created_at
    });
    if args.iter().any(|arg| arg == "--json") {
        return match serde_json::to_string_pretty(&value) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => work_error(error),
        };
    }
    if result_only {
        println!("status: {:?}", exchange.status);
        if let Some(error) = &exchange.error {
            println!("error: {error}");
        }
        println!("{response}");
        return ExitCode::SUCCESS;
    }
    println!("exchange: {}", exchange.id);
    println!("status: {:?}", exchange.status);
    println!("recipient: {}", recipient.as_deref().unwrap_or("-"));
    println!(
        "provider: {} / {}",
        exchange.provider,
        profile.as_deref().unwrap_or("-")
    );
    println!("model: {}", exchange.model.as_deref().unwrap_or("-"));
    println!("execution: {:?}", exchange.execution_mode);
    println!("context: {}", exchange.context_fingerprint);
    println!("context items: {}", exchange.context_items.len());
    println!("context bytes: {}", exchange.context_bytes);
    println!("duration: {} ms", exchange.duration_ms.unwrap_or(0));
    println!("\nREQUEST\n{request}\n\nRESPONSE\n{response}");
    if let Some(error) = exchange.error {
        println!("\nERROR\n{error}");
    }
    ExitCode::SUCCESS
}

fn find_exchange(store: &FeltDbWorkStore, id: &str) -> combe_state::Result<AiProviderExchange> {
    for conversation in store.ai_conversation_ids()? {
        if let Some(exchange) = store
            .ai_exchanges(&conversation)?
            .into_iter()
            .find(|exchange| exchange.id == id)
        {
            return Ok(exchange);
        }
    }
    Err(combe_state::StateError::NotFound {
        entity_type: "provider exchange".into(),
        id: id.into(),
    })
}

fn ai_message_content(events: &[combe_state::AiContextEvent], id: &str) -> Option<String> {
    events.iter().find_map(|event| match &event.event {
        AiContextEventData::MessageCreated { message } if message.id == id => {
            Some(message.content.clone())
        }
        _ => None,
    })
}

fn work_context(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work context needs an id");
        return ExitCode::from(2);
    };
    match store.context(&id).and_then(|context| {
        let assignment = context.active_assignments.first().cloned();
        ContextPackage::from_context(context, assignment)
            .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))
    }) {
        Ok(package)
            if option(args, "--format").as_deref() == Some("json")
                || args.iter().any(|arg| arg == "--json") =>
        {
            match serde_json::to_string_pretty(&package) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => work_error(error),
            }
        }
        Ok(package)
            if option(args, "--format")
                .as_deref()
                .is_none_or(|value| value == "text") =>
        {
            print!("{}", package.text());
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("combe: context format must be text or json");
            ExitCode::from(2)
        }
        Err(error) => work_error(error),
    }
}

fn protocol_action(value: Option<String>) -> Result<WorkAction, combe_state::StateError> {
    match value.as_deref().unwrap_or("inspect") {
        "inspect" => Ok(WorkAction::Inspect),
        "propose" => Ok(WorkAction::Propose),
        "review" => Ok(WorkAction::Review),
        "decide" => Ok(WorkAction::Decide),
        "assign" => Ok(WorkAction::Assign),
        "execute" => Ok(WorkAction::Execute),
        "report" => Ok(WorkAction::Report),
        value => Err(combe_state::StateError::InvalidEntity(format!(
            "unknown Work action {value}"
        ))),
    }
}

fn work_protocol(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work protocol needs an id");
        return ExitCode::from(2);
    };
    let result = (|| {
        let context = store.context(&id)?;
        let assignment = context.active_assignments.first().cloned();
        let mut package = ContextPackage::from_context(context, assignment)
            .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
        let requested_participant = if let Some(value) = option(args, "--participant") {
            let participant = participant(store, &id, &value)?;
            package.request.participant_id = participant.id.clone();
            participant
        } else {
            store
                .load_participant(&package.request.participant_id)?
                .ok_or_else(|| {
                    combe_state::StateError::InvalidEntity("missing participant".into())
                })?
        };
        package.request.requested_action = protocol_action(option(args, "--action"))?;
        package.request.validate(&requested_participant)?;
        Ok::<_, combe_state::StateError>(package)
    })();
    match result {
        Ok(package)
            if option(args, "--format").as_deref() == Some("json")
                || args.iter().any(|arg| arg == "--json") =>
        {
            match serde_json::to_string_pretty(&package) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => work_error(error),
            }
        }
        Ok(package)
            if option(args, "--format")
                .as_deref()
                .is_none_or(|format| format == "text") =>
        {
            print!("{}", package.text());
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("combe: protocol format must be text or json");
            ExitCode::from(2)
        }
        Err(error) => work_error(error),
    }
}

fn chatgpt_participant(store: &impl WorkStore, id: &WorkId) -> combe_state::Result<Participant> {
    if let Some(mut participant) = store.find_participant(id, "ChatGPT")? {
        let expected = combe_state::ParticipantCapabilities {
            can_propose: true,
            can_review: true,
            can_execute: false,
            can_decide: false,
        };
        if participant.capabilities != expected {
            participant.capabilities = expected;
            store.add_participant(participant.clone())?;
        }
        return Ok(participant);
    }
    let mut participant = Participant::new(id.clone(), ParticipantKind::Agent, "ChatGPT".into());
    participant.capabilities.can_execute = false;
    participant.capabilities.can_decide = false;
    store.add_participant(participant.clone())?;
    Ok(participant)
}

fn chatgpt_conversation(
    store: &impl WorkStore,
    id: &WorkId,
    participant: &Participant,
    external_id: &str,
    title: Option<String>,
) -> combe_state::Result<ConversationRef> {
    if let Some(link) = store.conversations(id)?.into_iter().find(|link| {
        link.participant_id == participant.id
            && link.conversation.provider == ConversationProvider::ChatGpt
            && link.conversation.id == external_id
    }) {
        return Ok(link.conversation);
    }
    let reference = ConversationRef {
        id: external_id.into(),
        provider: ConversationProvider::ChatGpt,
        title: title.clone(),
    };
    store.link_conversation(WorkConversation {
        id: ConversationId::new(),
        work_id: id.clone(),
        conversation: reference.clone(),
        participant_id: participant.id.clone(),
        label: title,
        created_at: Utc::now(),
    })?;
    Ok(reference)
}

fn work_chatgpt(store: &impl WorkStore, args: &[String]) -> ExitCode {
    if args.first().is_some_and(|arg| arg == "import") {
        return work_chatgpt_import(store, &args[1..]);
    }
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work chatgpt needs a Work id");
        return ExitCode::from(2);
    };
    let result = (|| {
        let participant = chatgpt_participant(store, &id)?;
        if let Some(external_id) = option(args, "--conversation") {
            chatgpt_conversation(
                store,
                &id,
                &participant,
                &external_id,
                option(args, "--title"),
            )?;
        }
        let context = store.context(&id)?;
        let action = protocol_action(option(args, "--action"))?;
        let prepared = ChatGptAdapter
            .prepare_context(context, &participant, action)
            .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
        prepared.package.request.validate(&participant)?;
        Ok::<_, combe_state::StateError>(prepared)
    })();
    match result {
        Ok(prepared) => {
            print!("{}", prepared.rendered);
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn chatgpt_contribution_kind(value: Option<String>) -> combe_state::Result<ContributionKind> {
    match value.as_deref() {
        Some("message") => Ok(ContributionKind::Message),
        Some("proposal") => Ok(ContributionKind::Proposal),
        Some("review") => Ok(ContributionKind::Review),
        Some("decision") => Ok(ContributionKind::Decision),
        Some(value) => Err(combe_state::StateError::InvalidEntity(format!(
            "unsupported ChatGPT contribution kind {value}"
        ))),
        None => Err(combe_state::StateError::InvalidEntity(
            "ChatGPT import needs --kind".into(),
        )),
    }
}

fn structured_revision(input: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()?
        .get("based_on_revision")?
        .as_u64()
}

fn work_chatgpt_import(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work chatgpt import needs a Work id");
        return ExitCode::from(2);
    };
    let result = (|| {
        let participant = store.find_participant(&id, "ChatGPT")?.ok_or_else(|| {
            combe_state::StateError::InvalidEntity(
                "no ChatGPT participant is registered; prepare context first".into(),
            )
        })?;
        let kind = chatgpt_contribution_kind(option(args, "--kind"))?;
        let input = stdin_content()
            .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
        let revision = option(args, "--revision")
            .map(|value| {
                value.parse::<u64>().map_err(|_| {
                    combe_state::StateError::InvalidEntity("revision must be an integer".into())
                })
            })
            .transpose()?
            .or_else(|| structured_revision(&input))
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity(
                    "plain-text ChatGPT import needs --revision from the prepared context".into(),
                )
            })?;
        let action = match kind {
            ContributionKind::Proposal => WorkAction::Propose,
            ContributionKind::Review => WorkAction::Review,
            ContributionKind::Message => WorkAction::Inspect,
            ContributionKind::Decision => WorkAction::Decide,
            _ => {
                return Err(combe_state::StateError::InvalidEntity(
                    "unsupported ChatGPT contribution kind".into(),
                ));
            }
        };
        let source = if let Some(external_id) = option(args, "--conversation") {
            let reference = store
                .conversations(&id)?
                .into_iter()
                .find(|link| {
                    link.participant_id == participant.id
                        && link.conversation.provider == ConversationProvider::ChatGpt
                        && link.conversation.id == external_id
                })
                .map(|link| link.conversation)
                .ok_or_else(|| {
                    combe_state::StateError::InvalidEntity(
                        "ChatGPT conversation is not linked; prepare context with --conversation first"
                            .into(),
                    )
                })?;
            TurnOrigin::ExternalConversation(reference)
        } else {
            TurnOrigin::Local
        };
        let context = store.context(&id)?;
        let requested_execution = option(args, "--execution").map(ExecutionId);
        let execution = requested_execution
            .as_ref()
            .and_then(|execution_id| {
                context
                    .executions
                    .iter()
                    .find(|execution| execution.id == *execution_id)
            })
            .or_else(|| {
                (kind == ContributionKind::Review)
                    .then(|| context.executions.last())
                    .flatten()
            });
        if kind == ContributionKind::Review
            && execution.is_none()
            && option(args, "--proposal").is_none()
        {
            return Err(combe_state::StateError::InvalidEntity(
                "ChatGPT review needs an execution or proposal reference".into(),
            ));
        }
        let request = combe_state::WorkRequest {
            work_id: id,
            participant_id: participant.id.clone(),
            context_version: WORK_CONTEXT_VERSION,
            revision,
            requested_action: action,
        };
        let contribution = ChatGptAdapter
            .ingest_contribution(ExternalContribution {
                request,
                participant,
                source,
                explicit_kind: kind,
                title: option(args, "--title"),
                proposal_id: option(args, "--proposal").map(ProposalId),
                assignment_id: execution.map(|execution| execution.assignment_id.clone()),
                execution_id: execution.map(|execution| execution.id.clone()),
                input,
            })
            .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
        store.accept_contribution(contribution)
    })();
    match result {
        Ok(accepted) => {
            match accepted {
                ContributionAcceptance::Turn(id) => println!("{id}"),
                ContributionAcceptance::Proposal(id) => println!("{id}"),
                ContributionAcceptance::ProposalReview(id) => println!("{id}"),
                ContributionAcceptance::ExecutionReview(id) => println!("{id}"),
                ContributionAcceptance::Decision(id) => println!("{id}"),
            }
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_review(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work review needs a Work id");
        return ExitCode::from(2);
    };
    let (Some(participant_value), Some(execution_value)) =
        (option(args, "--participant"), option(args, "--execution"))
    else {
        eprintln!("combe: work review needs --participant and --execution");
        return ExitCode::from(2);
    };
    let result = (|| {
        let reviewer = participant(store, &id, &participant_value)?;
        let execution_id = ExecutionId(execution_value);
        let execution = store.load_execution(&execution_id)?.ok_or_else(|| {
            combe_state::StateError::NotFound {
                entity_type: "execution".into(),
                id: execution_id.0.clone(),
            }
        })?;
        let revision = option(args, "--revision")
            .map(|value| {
                value.parse::<u64>().map_err(|_| {
                    combe_state::StateError::InvalidEntity("revision must be an integer".into())
                })
            })
            .transpose()?
            .unwrap_or(store.current_revision()?);
        let source = if let Some(conversation_id) = option(args, "--conversation") {
            store
                .conversations(&id)?
                .into_iter()
                .find(|link| {
                    link.participant_id == reviewer.id && link.conversation.id == conversation_id
                })
                .map(|link| TurnOrigin::ExternalConversation(link.conversation))
                .ok_or_else(|| {
                    combe_state::StateError::InvalidEntity(
                        "conversation is not linked to the reviewing participant".into(),
                    )
                })?
        } else {
            TurnOrigin::Local
        };
        let mut content = String::new();
        std::io::stdin()
            .read_to_string(&mut content)
            .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
        store.accept_contribution(WorkContribution {
            work_id: id,
            participant_id: reviewer.id,
            context_version: WORK_CONTEXT_VERSION,
            based_on_revision: revision,
            source,
            kind: ContributionKind::Review,
            content,
            title: None,
            rationale: None,
            review_outcome: None,
            proposal_id: None,
            assignment_id: Some(execution.assignment_id),
            execution_id: Some(execution.id),
            created_at: Utc::now(),
        })
    })();
    match result {
        Ok(ContributionAcceptance::ExecutionReview(id)) => {
            println!("{id}");
            ExitCode::SUCCESS
        }
        Ok(_) => unreachable!(),
        Err(error) => work_error(error),
    }
}

fn human(store: &impl WorkStore, id: &WorkId) -> combe_state::Result<Participant> {
    store.find_participant(id, "Human")?.ok_or_else(|| {
        combe_state::StateError::InvalidEntity("work has no Human participant".into())
    })
}

fn work_decision(store: &impl WorkStore, args: &[String]) -> ExitCode {
    if args.len() < 2 {
        eprintln!("combe: work decision needs an id and statement");
        return ExitCode::from(2);
    }
    let id = WorkId(args[0].clone());
    let result = (|| {
        let participant = human(store, &id)?;
        let decision = WorkDecision {
            id: Default::default(),
            work_id: id.clone(),
            statement: args[1].clone(),
            rationale: option(args, "--rationale"),
            decided_by: participant.id.clone(),
            created_at: Utc::now(),
            proposal_id: None,
            review_id: None,
        };
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: id,
            participant_id: participant.id,
            kind: TurnKind::Decision,
            content: decision.statement.clone(),
            created_at: decision.created_at,
            assignment_id: None,
            execution_id: None,
            origin: combe_state::TurnOrigin::Local,
        };
        store.record_decision(decision, Some(turn))
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn work_proposal(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("combe: work proposal needs a command");
        return ExitCode::from(2);
    };
    match command {
        "create" => work_proposal_create(store, &args[1..]),
        "list" => work_proposal_list(store, &args[1..]),
        "show" => work_proposal_show(store, &args[1..]),
        "approve" => work_proposal_review(store, &args[1..], ReviewOutcome::Approve),
        "reject" => work_proposal_review(store, &args[1..], ReviewOutcome::Reject),
        "request-changes" => work_proposal_review(store, &args[1..], ReviewOutcome::RequestChanges),
        other => {
            eprintln!("combe: unknown work proposal command '{other}'");
            ExitCode::from(2)
        }
    }
}

fn work_proposal_create(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(work_id) = parse_work_id(args) else {
        eprintln!("combe: work proposal create needs a work id");
        return ExitCode::from(2);
    };
    let (Some(participant_value), Some(title)) =
        (option(args, "--participant"), option(args, "--title"))
    else {
        eprintln!("combe: proposal create needs --participant and --title");
        return ExitCode::from(2);
    };
    let statement = match stdin_content() {
        Ok(statement) if !statement.trim().is_empty() => statement,
        Ok(_) => {
            eprintln!("combe: proposal create needs a statement on stdin");
            return ExitCode::from(2);
        }
        Err(error) => return work_error(error),
    };
    let proposal_id = ProposalId::new();
    let result = (|| {
        let participant = participant(store, &work_id, &participant_value)?;
        let origin = if let Some(conversation_id) = option(args, "--conversation") {
            let link = store
                .conversations(&work_id)?
                .into_iter()
                .find(|link| {
                    link.participant_id == participant.id && link.conversation.id == conversation_id
                })
                .ok_or_else(|| {
                    combe_state::StateError::InvalidEntity(format!(
                        "conversation not linked to participant: {conversation_id}"
                    ))
                })?;
            TurnOrigin::ExternalConversation(link.conversation)
        } else {
            TurnOrigin::Local
        };
        let now = Utc::now();
        store.create_proposal(WorkProposal {
            id: proposal_id.clone(),
            work_id,
            proposed_by: participant.id,
            title,
            statement,
            rationale: option(args, "--rationale"),
            status: if args.iter().any(|arg| arg == "--draft") {
                ProposalStatus::Draft
            } else {
                ProposalStatus::Proposed
            },
            origin,
            created_at: now,
            updated_at: now,
        })
    })();
    match result {
        Ok(()) => {
            let status = if args.iter().any(|arg| arg == "--draft") {
                "draft"
            } else {
                "proposed"
            };
            println!("{proposal_id}\t{status}");
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_proposal_list(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(work_id) = parse_work_id(args) else {
        eprintln!("combe: work proposal list needs a work id");
        return ExitCode::from(2);
    };
    match store.proposals(&work_id) {
        Ok(proposals) => {
            for proposal in proposals {
                println!("{}\t{:?}\t{}", proposal.id, proposal.status, proposal.title);
            }
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_proposal_show(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = args.first().map(|id| ProposalId(id.clone())) else {
        eprintln!("combe: work proposal show needs a proposal id");
        return ExitCode::from(2);
    };
    match store.load_proposal(&id) {
        Ok(Some(proposal)) => {
            println!("{} [{:?}]", proposal.title, proposal.status);
            println!("id: {}", proposal.id);
            println!("work: {}", proposal.work_id);
            println!("proposed by: {}", proposal.proposed_by);
            println!("\n{}", proposal.statement);
            ExitCode::SUCCESS
        }
        Ok(None) => work_error(combe_state::StateError::NotFound {
            entity_type: "proposal".into(),
            id: id.0,
        }),
        Err(error) => work_error(error),
    }
}

fn work_proposal_review(
    store: &impl WorkStore,
    args: &[String],
    outcome: ReviewOutcome,
) -> ExitCode {
    let Some(proposal_id) = args.first().map(|id| ProposalId(id.clone())) else {
        eprintln!("combe: proposal review needs a proposal id");
        return ExitCode::from(2);
    };
    let Some(reviewer) = option(args, "--by") else {
        eprintln!("combe: proposal review needs --by");
        return ExitCode::from(2);
    };
    let result = (|| {
        let proposal = store.load_proposal(&proposal_id)?.ok_or_else(|| {
            combe_state::StateError::NotFound {
                entity_type: "proposal".into(),
                id: proposal_id.0.clone(),
            }
        })?;
        let reviewer = participant(store, &proposal.work_id, &reviewer)?;
        store.review_proposal(ProposalReview {
            id: ReviewId::new(),
            proposal_id,
            reviewed_by: reviewer.id,
            outcome,
            comment: option(args, "--comment"),
            origin: TurnOrigin::Local,
            created_at: Utc::now(),
        })
    })();
    match result {
        Ok((proposal, _)) => {
            println!("{}\t{:?}", proposal.id, proposal.status);
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_assign(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id_value) = args.first() else {
        eprintln!("combe: work assign needs a work id");
        return ExitCode::from(2);
    };
    let target =
        option(args, "--to").or_else(|| args.get(1).filter(|v| !v.starts_with("--")).cloned());
    let instruction = option(args, "--instruction")
        .or_else(|| args.get(2).filter(|v| !v.starts_with("--")).cloned());
    let (Some(target), Some(instruction)) = (target, instruction) else {
        eprintln!("combe: work assign needs --to and --instruction");
        return ExitCode::from(2);
    };
    let id = WorkId(id_value.clone());
    let result = (|| {
        let from = human(store, &id)?;
        let to = match store.find_participant(&id, &target)? {
            Some(participant) => participant,
            None => {
                let participant =
                    Participant::new(id.clone(), ParticipantKind::Agent, target.clone());
                store.add_participant(participant.clone())?;
                participant
            }
        };
        store.add_assignment(WorkAssignment {
            id: AssignmentId::new(),
            work_id: id,
            from_participant_id: from.id,
            to_participant_id: to.id,
            instruction,
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
            proposal_id: option(args, "--proposal").map(ProposalId),
        })
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn participant(
    store: &impl WorkStore,
    id: &WorkId,
    value: &str,
) -> combe_state::Result<Participant> {
    store
        .participants(id)?
        .into_iter()
        .find(|participant| {
            participant.id.0 == value || participant.name.eq_ignore_ascii_case(value)
        })
        .ok_or_else(|| {
            combe_state::StateError::InvalidEntity(format!("participant not found: {value}"))
        })
}

fn work_turn(store: &impl WorkStore, args: &[String]) -> ExitCode {
    work_contribute(store, args)
}

fn parse_turn_kind(args: &[String]) -> Result<TurnKind, String> {
    match option(args, "--kind").as_deref().unwrap_or("message") {
        "message" => Ok(TurnKind::Message),
        "analysis" => Ok(TurnKind::Analysis),
        "review" => Ok(TurnKind::Review),
        "implementation" => Ok(TurnKind::Implementation),
        "decision" => Ok(TurnKind::Decision),
        other => Err(format!("invalid contribution kind '{other}'")),
    }
}

fn stdin_content() -> Result<String, std::io::Error> {
    let mut content = String::new();
    std::io::stdin().read_to_string(&mut content)?;
    Ok(content)
}

fn work_contribute(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work contribute needs an id");
        return ExitCode::from(2);
    };
    let Some(participant_value) = option(args, "--participant") else {
        eprintln!("combe: work contribute needs --participant");
        return ExitCode::from(2);
    };
    let kind = match parse_turn_kind(args) {
        Ok(kind) => kind,
        Err(error) => {
            eprintln!("combe: {error}");
            return ExitCode::from(2);
        }
    };
    let content = match stdin_content() {
        Ok(content) => content,
        Err(error) => return work_error(error),
    };
    if content.trim().is_empty() {
        eprintln!("combe: work contribute needs content on stdin");
        return ExitCode::from(2);
    }
    let turn_id = TurnId::new();
    let result = participant(store, &id, &participant_value).and_then(|participant| {
        store.add_turn(WorkTurn {
            id: turn_id.clone(),
            work_id: id,
            participant_id: participant.id,
            kind,
            content,
            created_at: Utc::now(),
            assignment_id: None,
            execution_id: None,
            origin: TurnOrigin::Local,
        })
    });
    match result {
        Ok(()) => {
            println!("{turn_id}");
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn conversation_provider(value: &str) -> ConversationProvider {
    match value.to_ascii_lowercase().as_str() {
        "chatgpt" | "chat-gpt" => ConversationProvider::ChatGpt,
        "claude" | "claude-code" => ConversationProvider::Claude,
        "codex" => ConversationProvider::Codex,
        _ => ConversationProvider::Other(value.to_string()),
    }
}

fn work_import(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work import needs an id");
        return ExitCode::from(2);
    };
    let (Some(participant_value), Some(conversation_id)) = (
        option(args, "--participant"),
        option(args, "--conversation"),
    ) else {
        eprintln!("combe: work import needs --participant and --conversation");
        return ExitCode::from(2);
    };
    let kind = match parse_turn_kind(args) {
        Ok(kind) => kind,
        Err(error) => {
            eprintln!("combe: {error}");
            return ExitCode::from(2);
        }
    };
    let content = match stdin_content() {
        Ok(content) => content,
        Err(error) => return work_error(error),
    };
    let result = (|| {
        let provider = conversation_provider(
            option(args, "--provider")
                .as_deref()
                .unwrap_or(&participant_value),
        );
        let participant = match store.participants(&id)?.into_iter().find(|candidate| {
            candidate.id.0 == participant_value
                || candidate.name.eq_ignore_ascii_case(&participant_value)
        }) {
            Some(participant) => participant,
            None => {
                let mut participant = Participant::new(
                    id.clone(),
                    ParticipantKind::Agent,
                    participant_value.clone(),
                );
                if provider == ConversationProvider::ChatGpt {
                    participant.capabilities.can_execute = false;
                    participant.capabilities.can_decide = false;
                }
                store.add_participant(participant.clone())?;
                participant
            }
        };
        let requested_reference = ConversationRef {
            id: conversation_id,
            provider,
            title: option(args, "--title"),
        };
        let existing = store.conversations(&id)?.into_iter().find(|link| {
            link.participant_id == participant.id
                && link.conversation.id == requested_reference.id
                && link.conversation.provider == requested_reference.provider
        });
        let reference = existing
            .as_ref()
            .map(|link| link.conversation.clone())
            .unwrap_or_else(|| requested_reference.clone());
        let conversation = existing.unwrap_or_else(|| WorkConversation {
            id: ConversationId::new(),
            work_id: id.clone(),
            conversation: requested_reference,
            participant_id: participant.id.clone(),
            label: option(args, "--title"),
            created_at: Utc::now(),
        });
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: id,
            participant_id: participant.id,
            kind,
            content,
            created_at: Utc::now(),
            assignment_id: None,
            execution_id: None,
            origin: TurnOrigin::ExternalConversation(reference),
        };
        let turn_id = turn.id.clone();
        store.import_contribution(conversation, turn)?;
        Ok::<TurnId, combe_state::StateError>(turn_id)
    })();
    match result {
        Ok(turn_id) => {
            println!("{turn_id}");
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn assignment(store: &impl WorkStore, args: &[String]) -> combe_state::Result<WorkAssignment> {
    let id = args.first().ok_or_else(|| {
        combe_state::StateError::InvalidEntity("execution command needs an assignment id".into())
    })?;
    store
        .load_assignment(&AssignmentId(id.clone()))?
        .ok_or_else(|| combe_state::StateError::NotFound {
            entity_type: "assignment".into(),
            id: id.clone(),
        })
}

fn work_start(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let result = (|| {
        let assignment = assignment(store, args)?;
        let provider = option(args, "--provider").ok_or_else(|| {
            combe_state::StateError::InvalidEntity("work start needs --provider".into())
        })?;
        let now = Utc::now();
        store.start_execution(WorkExecution {
            id: ExecutionId::new(),
            work_id: assignment.work_id,
            assignment_id: assignment.id,
            participant_id: assignment.to_participant_id,
            provider,
            provider_execution_id: option(args, "--provider-id"),
            status: ExecutionStatus::Started,
            started_at: now,
            completed_at: None,
            heartbeat_at: Some(now),
            result_id: None,
            failure: None,
            created_at: now,
            updated_at: now,
        })
    })();
    match result {
        Ok(execution) => {
            println!("{}\t{:?}", execution.id, execution.status);
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_execution_status(store: &impl WorkStore, args: &[String]) -> ExitCode {
    match assignment(store, args).and_then(|assignment| {
        store
            .execution_for_assignment(&assignment.id)
            .map(|execution| (assignment, execution))
    }) {
        Ok((assignment, Some(execution))) => {
            println!("assignment: {} [{:?}]", assignment.id, assignment.status);
            println!("execution: {} [{:?}]", execution.id, execution.status);
            println!("provider: {}", execution.provider);
            if let Some(id) = execution.provider_execution_id {
                println!("provider execution: {id}");
            }
            if let Some(failure) = execution.failure {
                println!("failure: {failure}");
            }
            ExitCode::SUCCESS
        }
        Ok((assignment, None)) => {
            println!("assignment: {} [{:?}]", assignment.id, assignment.status);
            println!("execution: not started");
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_heartbeat(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let result = assignment(store, args).and_then(|assignment| {
        store
            .execution_for_assignment(&assignment.id)?
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("assignment has not started".into())
            })
            .and_then(|execution| store.heartbeat_execution(&execution.id, Utc::now()))
    });
    match result {
        Ok(execution) => {
            println!("{}\t{:?}", execution.id, execution.status);
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_finish_execution(
    store: &impl WorkStore,
    args: &[String],
    status: ExecutionStatus,
) -> ExitCode {
    let result = (|| {
        let assignment = assignment(store, args)?;
        let mut execution = store
            .execution_for_assignment(&assignment.id)?
            .ok_or_else(|| {
                combe_state::StateError::InvalidEntity("assignment has not started".into())
            })?;
        let now = Utc::now();
        execution.status = status.clone();
        execution.completed_at = Some(now);
        execution.updated_at = now;
        execution.failure = match status {
            ExecutionStatus::Failed => {
                option(args, "--failure").or_else(|| Some("provider reported failure".into()))
            }
            ExecutionStatus::Cancelled => option(args, "--reason"),
            _ => None,
        };
        if status == ExecutionStatus::Cancelled {
            return store.finish_execution(execution, None, None, Vec::new());
        }
        let mut output = String::new();
        std::io::stdin()
            .read_to_string(&mut output)
            .map_err(|error| combe_state::StateError::InvalidEntity(error.to_string()))?;
        if output.trim().is_empty() {
            output = execution
                .failure
                .clone()
                .unwrap_or_else(|| "Execution completed".into());
        }
        output = bounded_execution_output(output);
        let result = ParticipantResult {
            id: uuid::Uuid::new_v4().to_string(),
            work_id: assignment.work_id.clone(),
            participant_id: assignment.to_participant_id.clone(),
            assignment_id: assignment.id.clone(),
            execution_id: execution.id.0.clone(),
            exit_status: (status == ExecutionStatus::Completed).then_some(0),
            summary: output.lines().next().map(str::to_owned),
            output: Some(output.clone()),
            created_at: now,
        };
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: assignment.work_id,
            participant_id: assignment.to_participant_id,
            kind: TurnKind::Implementation,
            content: output,
            created_at: now,
            assignment_id: Some(assignment.id),
            execution_id: Some(execution.id.0.clone()),
            origin: TurnOrigin::Local,
        };
        store.finish_execution(execution, Some(result), Some(turn), Vec::new())
    })();
    match result {
        Ok(execution) => {
            println!("{}\t{:?}", execution.id, execution.status);
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
}

fn work_handoff(store: &impl WorkStore, args: &[String]) -> ExitCode {
    let Some(id) = parse_work_id(args) else {
        eprintln!("combe: work handoff needs an id");
        return ExitCode::from(2);
    };
    let (Some(target), Some(provider_name)) = (option(args, "--to"), option(args, "--provider"))
    else {
        eprintln!("combe: work handoff needs --to and --provider");
        return ExitCode::from(2);
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let participant = participant(store, &id, &target)?;
        let provider = LocalProvider::parse(&provider_name)?;
        let adapter = LocalCliAdapter::discover(provider)?;
        let context = store.context(&id)?;
        let assignment = context
            .active_assignments
            .iter()
            .find(|assignment| {
                assignment.to_participant_id == participant.id
                    && assignment.status == AssignmentStatus::Pending
            })
            .cloned()
            .ok_or_else(|| format!("no pending assignment for participant {target}"))?;
        let prepared = adapter.prepare(context, assignment.clone(), &participant)?;
        let started_at = Utc::now();
        let mut canonical = store.start_execution(WorkExecution {
            id: ExecutionId::new(),
            work_id: id.clone(),
            assignment_id: assignment.id.clone(),
            participant_id: participant.id.clone(),
            provider: provider_name.clone(),
            provider_execution_id: None,
            status: ExecutionStatus::Started,
            started_at,
            completed_at: None,
            heartbeat_at: Some(started_at),
            result_id: None,
            failure: None,
            created_at: started_at,
            updated_at: started_at,
        })?;
        let provider_result = adapter.launch(prepared);
        let (exit_status, mut output, terminal_status, failure, provider_execution_id) =
            match provider_result {
                Ok(result) => {
                    let mut output = result.stdout.trim().to_string();
                    if !result.stderr.trim().is_empty() {
                        if !output.is_empty() {
                            output.push_str("\n\nSTDERR\n");
                        }
                        output.push_str(result.stderr.trim());
                    }
                    let status = if result.exit_status == Some(0) {
                        ExecutionStatus::Completed
                    } else {
                        ExecutionStatus::Failed
                    };
                    let failure = (status == ExecutionStatus::Failed)
                        .then(|| format!("provider exited with {:?}", result.exit_status));
                    (
                        result.exit_status,
                        output,
                        status,
                        failure,
                        Some(result.execution_id),
                    )
                }
                Err(error) => (
                    None,
                    error.to_string(),
                    ExecutionStatus::Failed,
                    Some(error.to_string()),
                    None,
                ),
            };
        if output.trim().is_empty() {
            output = failure
                .clone()
                .unwrap_or_else(|| "Execution completed".into());
        }
        output = bounded_execution_output(output);
        let now = Utc::now();
        canonical.status = terminal_status;
        canonical.completed_at = Some(now);
        canonical.updated_at = now;
        canonical.failure = failure;
        canonical.provider_execution_id = provider_execution_id;
        let result = ParticipantResult {
            id: uuid::Uuid::new_v4().to_string(),
            work_id: id.clone(),
            participant_id: participant.id.clone(),
            assignment_id: assignment.id.clone(),
            execution_id: canonical.id.0.clone(),
            exit_status,
            summary: output.lines().next().map(str::to_owned),
            output: (!output.is_empty()).then(|| output.clone()),
            created_at: now,
        };
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: id.clone(),
            participant_id: participant.id.clone(),
            kind: TurnKind::Implementation,
            content: output,
            created_at: now,
            assignment_id: Some(assignment.id.clone()),
            execution_id: Some(canonical.id.0.clone()),
            origin: combe_state::TurnOrigin::Local,
        };
        let artifacts = git_artifacts(&context_worktree(store, &id)?, &id, &participant.id, now);
        store.finish_execution(canonical, Some(result), Some(turn), artifacts)?;
        Ok(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn context_worktree(store: &impl WorkStore, id: &WorkId) -> combe_state::Result<String> {
    let workspace_id = store
        .load_work(id)?
        .map(|work| work.workspace_id)
        .ok_or_else(|| combe_state::StateError::NotFound {
            entity_type: "work".into(),
            id: id.0.clone(),
        })?;
    Ok(FeltDbWorkspaceStore::for_combe()
        .ok()
        .and_then(|store| store.load_workspace(&workspace_id).ok().flatten())
        .map(|workspace| workspace.path)
        .unwrap_or(workspace_id))
}

fn git_artifacts(
    worktree: &str,
    work_id: &WorkId,
    participant_id: &combe_state::ParticipantId,
    created_at: chrono::DateTime<Utc>,
) -> Vec<WorkArtifact> {
    let mut artifacts = Vec::new();
    if let Ok(output) = Command::new("git")
        .args(["-C", worktree, "status", "--porcelain=v1"])
        .output()
        && output.status.success()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let path = line
                .get(3..)
                .unwrap_or("")
                .split(" -> ")
                .last()
                .unwrap_or("")
                .trim();
            if !path.is_empty() {
                artifacts.push(WorkArtifact {
                    id: ArtifactId::new(),
                    work_id: work_id.clone(),
                    kind: ArtifactKind::File,
                    path: Some(path.into()),
                    description: Some("Changed during participant execution".into()),
                    created_by: participant_id.clone(),
                    created_at,
                });
            }
        }
    }
    if let Ok(output) = Command::new("git")
        .args(["-C", worktree, "rev-parse", "HEAD"])
        .output()
        && output.status.success()
    {
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        artifacts.push(WorkArtifact {
            id: ArtifactId::new(),
            work_id: work_id.clone(),
            kind: ArtifactKind::Commit,
            path: None,
            description: Some(commit),
            created_by: participant_id.clone(),
            created_at,
        });
    }
    artifacts
}

fn work_status(store: &impl WorkStore, args: &[String]) -> ExitCode {
    if args.len() < 2 {
        eprintln!("combe: work status needs an id and status");
        return ExitCode::from(2);
    }
    let status = match args[1].as_str() {
        "active" => combe_state::WorkStatus::Active,
        "paused" => combe_state::WorkStatus::Paused,
        "completed" => combe_state::WorkStatus::Completed,
        "archived" => combe_state::WorkStatus::Archived,
        other => {
            eprintln!("combe: invalid work status '{other}'");
            return ExitCode::from(2);
        }
    };
    match store.set_work_status(&WorkId(args[0].clone()), status) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn work_error(error: impl std::fmt::Display) -> ExitCode {
    eprintln!("combe: {error}");
    ExitCode::FAILURE
}

fn launched_from_bundle() -> bool {
    let Some(exe) = std::env::args_os().next().map(PathBuf::from) else {
        return false;
    };
    exe.parent().and_then(Path::parent).is_some_and(|contents| {
        contents.file_name() == Some(OsStr::new("Contents"))
            && contents.join("Info.plist").is_file()
    })
}

fn print_usage() {
    print!("{USAGE}");
    if let Some(path) = state_path() {
        println!("\nState: {}", path.display());
    }
}

fn list() -> ExitCode {
    let Some(state) = read_state() else {
        return ExitCode::FAILURE;
    };
    if state.repos.is_empty() {
        println!("no repos registered — combe add <path>");
        return ExitCode::SUCCESS;
    }
    let found: Catalog = match catalog(&state) {
        Ok(found) => found,
        Err(err) => {
            eprintln!("combe: {err}");
            return ExitCode::FAILURE;
        }
    };
    for repo in &state.repos {
        let rows: Vec<_> = found
            .rows
            .iter()
            .filter(|row| row.repo_path == repo.path)
            .collect();
        if rows.is_empty() {
            println!("{}  (missing)", repo.path.display());
            continue;
        }
        println!("{}", repo.path.display());
        for row in rows {
            println!("  {:<28} {}", row.label(), row.path.display());
        }
    }
    ExitCode::SUCCESS
}

fn add(paths: &[String]) -> ExitCode {
    if paths.is_empty() {
        eprintln!("combe: add needs at least one path");
        return ExitCode::from(2);
    }
    edit(|state| {
        let mut failed = false;
        for path in paths {
            match add_repo(state, Path::new(path)) {
                Ok(resolved) => println!("added {}", resolved.display()),
                Err(err) => {
                    eprintln!("combe: {err}");
                    failed = true;
                }
            }
        }
        !failed
    })
}

fn remove(paths: &[String]) -> ExitCode {
    if paths.is_empty() {
        eprintln!("combe: remove needs at least one path");
        return ExitCode::from(2);
    }
    edit(|state| {
        let mut failed = false;
        for path in paths {
            match remove_repo(state, Path::new(path)) {
                Ok(true) => println!("removed {path}"),
                Ok(false) => {
                    eprintln!("combe: not registered: {path}");
                    failed = true;
                }
                Err(err) => {
                    eprintln!("combe: {err}");
                    failed = true;
                }
            }
        }
        !failed
    })
}

fn clean() -> ExitCode {
    edit(|state| {
        let cleaned = cleanup(state);
        if cleaned.is_empty() {
            println!("nothing to clean");
            return true;
        }
        for path in &cleaned.repos {
            println!("dropped repo {}", path.display());
        }
        true
    })
}

fn edit(apply: impl FnOnce(&mut State) -> bool) -> ExitCode {
    let (Some(file), Some(mut state)) = (state_path(), read_state()) else {
        return ExitCode::FAILURE;
    };
    let before = state.clone();
    let ok = apply(&mut state);
    if state != before
        && let Err(err) = save_state(&file, &state)
    {
        eprintln!("combe: {err}");
        return ExitCode::FAILURE;
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn read_state() -> Option<State> {
    let file: PathBuf = state_path()?;
    match load_state(&file) {
        Ok(state) => Some(state),
        Err(err) => {
            eprintln!("combe: {err}");
            None
        }
    }
}

fn open_workspace(paths: &[String]) -> ExitCode {
    if paths.is_empty() {
        eprintln!("combe: open needs a path");
        return ExitCode::from(2);
    }
    let path = &paths[0];
    match std::fs::canonicalize(path) {
        Ok(canonical) => {
            println!("Opening workspace: {}", canonical.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("combe: cannot access {}: {}", path, err);
            ExitCode::FAILURE
        }
    }
}

fn doctor() -> ExitCode {
    println!("Combe diagnostics:\n");

    match std::env::consts::ARCH {
        "aarch64" => println!("  Architecture: Apple Silicon (arm64)"),
        "x86_64" => println!("  Architecture: Intel (x86_64)"),
        other => println!("  Architecture: {}", other),
    }

    match which::which("git") {
        Ok(path) => match std::process::Command::new("git").arg("--version").output() {
            Ok(output) => {
                let version = String::from_utf8_lossy(&output.stdout);
                println!("  Git: {} ({})", version.trim(), path.display());
            }
            Err(_) => println!("  Git: found but cannot execute"),
        },
        Err(_) => println!("  Git: not found"),
    }

    match which::which("ghostty") {
        Ok(path) => println!("  Ghostty: {}", path.display()),
        Err(_) => println!("  Ghostty: not found in PATH"),
    }

    if let Some(state_path) = state_path() {
        match std::fs::metadata(&state_path) {
            Ok(meta) => {
                println!(
                    "  Catalog state: {} ({} bytes)",
                    state_path.display(),
                    meta.len()
                );
            }
            Err(_) => println!(
                "  Catalog state: {} (will be created)",
                state_path.display()
            ),
        }
    } else {
        println!("  Catalog state: cannot determine location");
    }

    let Some(state) = read_state() else {
        eprintln!("combe: failed to read catalog state");
        return ExitCode::FAILURE;
    };
    println!("  Registered repos: {}", state.repos.len());
    for repo in &state.repos {
        if repo.path.is_dir() {
            println!("    ✓ {}", repo.path.display());
        } else {
            println!("    ✗ {} (missing)", repo.path.display());
        }
    }

    println!("\n  Workspace state:");
    match FeltDbWorkspaceStore::for_combe() {
        Ok(store) => match store.load_state() {
            Ok(ws_state) => {
                println!("    Workspaces: {}", ws_state.workspaces.len());
                for ws in &ws_state.workspaces {
                    if std::path::Path::new(&ws.path).is_dir() {
                        println!("      ✓ {} ({})", ws.id, ws.path);
                    } else {
                        println!("      ✗ {} (missing: {})", ws.id, ws.path);
                    }
                }
                if let Some(selected) = &ws_state.selected_workspace_id {
                    println!("    Selected: {}", selected);
                }
            }
            Err(err) => println!("    Error reading state: {}", err),
        },
        Err(err) => println!("    Error: {}", err),
    }

    ExitCode::SUCCESS
}

fn version() -> ExitCode {
    let version = env!("CARGO_PKG_VERSION");
    println!("Combe {}", version);
    println!(
        "Built for: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    ExitCode::SUCCESS
}
