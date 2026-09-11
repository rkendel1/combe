use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::participant_adapter::{
    ContextPackage, LocalCliAdapter, LocalProvider, ParticipantAdapter,
};
use chrono::Utc;
use combe_catalog::{
    Catalog, State, add_repo, catalog, cleanup, load_state, remove_repo, save_state, state_path,
};
use combe_state::{
    ArtifactId, ArtifactKind, AssignmentId, AssignmentStatus, ConversationId, ConversationProvider,
    ConversationRef, FeltDbWorkStore, FeltDbWorkspaceStore, Participant, ParticipantKind,
    ParticipantResult, ProposalId, ProposalReview, ProposalStatus, ReviewId, ReviewOutcome, TurnId,
    TurnKind, TurnOrigin, Work, WorkArtifact, WorkAssignment, WorkConversation, WorkDecision,
    WorkId, WorkProposal, WorkStore, WorkTurn, WorkspaceStore,
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
  combe work list [--workspace <id>]
  combe work create <title> [--workspace <id>] [--objective <text>]
  combe work show <id>
  combe work context <id> [--format <text|json>]
  combe work decision <id> <statement> [--rationale <text>]
  combe work proposal create <work-id> --participant <name-or-id> --title <title>
  combe work proposal list <work-id>
  combe work proposal show <proposal-id>
  combe work proposal approve <proposal-id> --by <participant>
  combe work proposal reject <proposal-id> --by <participant>
  combe work proposal request-changes <proposal-id> --by <participant>
  combe work assign <id> --to <participant> [--proposal <id>] --instruction <text>
  combe work handoff <id> --to <participant> --provider <codex|claude>
  combe work turn <id> --participant <name-or-id> [--kind <kind>]
  combe work contribute <id> --participant <name-or-id> [--kind <kind>]
  combe work import <id> --participant <name-or-id> --conversation <id> [--provider <provider>] [--title <title>] [--kind <kind>]
  combe work status <id> <active|paused|completed|archived>
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
        "work" => Some(work(rest)),
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
        "context" => work_context(&store, &args[1..]),
        "decision" => work_decision(&store, &args[1..]),
        "proposal" => work_proposal(&store, &args[1..]),
        "assign" => work_assign(&store, &args[1..]),
        "handoff" => work_handoff(&store, &args[1..]),
        "turn" => work_turn(&store, &args[1..]),
        "contribute" => work_contribute(&store, &args[1..]),
        "import" => work_import(&store, &args[1..]),
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
            if let Some(objective) = context.work.objective {
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
            ExitCode::SUCCESS
        }
        Err(error) => work_error(error),
    }
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
        let participant = match store.participants(&id)?.into_iter().find(|candidate| {
            candidate.id.0 == participant_value
                || candidate.name.eq_ignore_ascii_case(&participant_value)
        }) {
            Some(participant) => participant,
            None => {
                let participant = Participant::new(
                    id.clone(),
                    ParticipantKind::Agent,
                    participant_value.clone(),
                );
                store.add_participant(participant.clone())?;
                participant
            }
        };
        let provider = conversation_provider(
            option(args, "--provider")
                .as_deref()
                .unwrap_or(&participant_value),
        );
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
        let mut assignment = context
            .active_assignments
            .iter()
            .find(|assignment| {
                assignment.to_participant_id == participant.id
                    && assignment.status == AssignmentStatus::Pending
            })
            .cloned()
            .ok_or_else(|| format!("no pending assignment for participant {target}"))?;
        let prepared = adapter.prepare(context, assignment.clone(), &participant)?;
        assignment = store.set_assignment_status(&assignment.id, AssignmentStatus::Active)?;
        let execution = match adapter.launch(prepared) {
            Ok(execution) => execution,
            Err(error) => {
                store.set_assignment_status(&assignment.id, AssignmentStatus::Failed)?;
                return Err(Box::new(error));
            }
        };
        assignment.status = if execution.exit_status == Some(0) {
            AssignmentStatus::Completed
        } else {
            AssignmentStatus::Failed
        };
        let mut output = execution.stdout.trim().to_string();
        if !execution.stderr.trim().is_empty() {
            if !output.is_empty() {
                output.push_str("\n\nSTDERR\n");
            }
            output.push_str(execution.stderr.trim());
        }
        let now = Utc::now();
        let result = ParticipantResult {
            id: uuid::Uuid::new_v4().to_string(),
            work_id: id.clone(),
            participant_id: participant.id.clone(),
            assignment_id: assignment.id.clone(),
            execution_id: execution.execution_id.clone(),
            exit_status: execution.exit_status,
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
            execution_id: Some(execution.execution_id),
            origin: combe_state::TurnOrigin::Local,
        };
        let artifacts = git_artifacts(&context_worktree(store, &id)?, &id, &participant.id, now);
        store.finish_assignment(assignment, result, turn, artifacts)?;
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
