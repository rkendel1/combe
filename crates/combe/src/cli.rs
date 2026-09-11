use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use combe_catalog::{
    Catalog, State, add_repo, catalog, cleanup, load_state, remove_repo, save_state, state_path,
};
use combe_state::{FileWorkspaceStore, WorkspaceStore};

const USAGE: &str = "\
combe — a worktree-aware terminal

Usage:
  combe list                Show registered repos and their worktrees
  combe add <path>...       Register repos
  combe remove <path>...    Unregister repos
  combe cleanup             Drop registered paths that no longer exist on disk
  combe open <path>         Open or focus a workspace
  combe doctor              Check Combe installation and state
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
        Ok(path) => {
            match std::process::Command::new("git")
                .arg("--version")
                .output()
            {
                Ok(output) => {
                    let version = String::from_utf8_lossy(&output.stdout);
                    println!("  Git: {} ({})", version.trim(), path.display());
                }
                Err(_) => println!("  Git: found but cannot execute"),
            }
        }
        Err(_) => println!("  Git: not found"),
    }

    match which::which("ghostty") {
        Ok(path) => println!("  Ghostty: {}", path.display()),
        Err(_) => println!("  Ghostty: not found in PATH"),
    }

    if let Some(state_path) = state_path() {
        match std::fs::metadata(&state_path) {
            Ok(meta) => {
                println!("  Catalog state: {} ({} bytes)", state_path.display(), meta.len());
            }
            Err(_) => println!("  Catalog state: {} (will be created)", state_path.display()),
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
    match FileWorkspaceStore::for_combe() {
        Ok(store) => {
            match store.load_state() {
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
            }
        }
        Err(err) => println!("    Error: {}", err),
    }

    ExitCode::SUCCESS
}

fn version() -> ExitCode {
    let version = env!("CARGO_PKG_VERSION");
    println!("Combe {}", version);
    println!("Built for: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    ExitCode::SUCCESS
}
