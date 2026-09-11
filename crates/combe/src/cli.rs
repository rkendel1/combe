use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::Utc;
use combe_catalog::{
    Catalog, State, add_repo, catalog, cleanup, load_state, remove_repo, save_state, state_path,
};
use combe_state::{
    AssignmentId, AssignmentStatus, FeltDbWorkStore, FeltDbWorkspaceStore, Participant,
    ParticipantKind, TurnId, TurnKind, Work, WorkAssignment, WorkDecision, WorkId, WorkStore,
    WorkTurn, WorkspaceStore,
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
  combe work context <id> [--json]
  combe work decision <id> <statement> [--rationale <text>]
  combe work assign <id> <participant> <instruction>
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
        "assign" => work_assign(&store, &args[1..]),
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
    match store.context(&id) {
        Ok(context) if args.iter().any(|arg| arg == "--json") => {
            match serde_json::to_string_pretty(&context) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(error) => work_error(error),
            }
        }
        Ok(context) => {
            println!("# {}\n", context.work.title);
            if let Some(objective) = context.work.objective {
                println!("Objective: {objective}\n");
            }
            println!(
                "Status: {:?}\nWorkspace: {}",
                context.work.status, context.work.workspace_id
            );
            if !context.participants.is_empty() {
                println!("\nParticipants");
                for participant in context.participants {
                    println!("- {} ({:?})", participant.name, participant.kind);
                }
            }
            if !context.active_assignments.is_empty() {
                println!("\nActive assignments");
                for assignment in context.active_assignments {
                    println!("- {:?}: {}", assignment.status, assignment.instruction);
                }
            }
            if !context.decisions.is_empty() {
                println!("\nDecisions");
                for decision in context.decisions {
                    println!("- {}", decision.statement);
                }
            }
            if !context.artifacts.is_empty() {
                println!("\nArtifacts");
                for artifact in context.artifacts {
                    println!(
                        "- {:?}: {}",
                        artifact.kind,
                        artifact.path.or(artifact.description).unwrap_or_default()
                    );
                }
            }
            if !context.recent_turns.is_empty() {
                println!("\nRecent turns");
                for turn in context.recent_turns {
                    println!("- {:?}: {}", turn.kind, turn.content);
                }
            }
            ExitCode::SUCCESS
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
        };
        let turn = WorkTurn {
            id: TurnId::new(),
            work_id: id,
            participant_id: participant.id,
            kind: TurnKind::Decision,
            content: decision.statement.clone(),
            created_at: decision.created_at,
        };
        store.record_decision(decision, Some(turn))
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
}

fn work_assign(store: &impl WorkStore, args: &[String]) -> ExitCode {
    if args.len() < 3 {
        eprintln!("combe: work assign needs an id, participant, and instruction");
        return ExitCode::from(2);
    }
    let id = WorkId(args[0].clone());
    let result = (|| {
        let from = human(store, &id)?;
        let to = match store.find_participant(&id, &args[1])? {
            Some(participant) => participant,
            None => {
                let participant =
                    Participant::new(id.clone(), ParticipantKind::Agent, args[1].clone());
                store.add_participant(participant.clone())?;
                participant
            }
        };
        store.add_assignment(WorkAssignment {
            id: AssignmentId::new(),
            work_id: id,
            from_participant_id: from.id,
            to_participant_id: to.id,
            instruction: args[2].clone(),
            status: AssignmentStatus::Pending,
            created_at: Utc::now(),
        })
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => work_error(error),
    }
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
