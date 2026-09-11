use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

pub const MAX_DIRECTORY_ENTRIES: usize = 250;
pub const MAX_TREE_ENTRIES: usize = 2000;
pub const MAX_FILE_PREVIEW_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFileState {
    Clean,
    Modified,
    Added,
    Deleted,
    Untracked,
    Ignored,
    Conflicted,
}

impl GitFileState {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Clean => "  ",
            Self::Modified => "M ",
            Self::Added => "A ",
            Self::Deleted => "D ",
            Self::Untracked => "??",
            Self::Ignored => "!!",
            Self::Conflicted => "UU",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub depth: usize,
    pub directory: bool,
    pub expanded: bool,
    pub git: GitFileState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilePreview {
    Text {
        path: PathBuf,
        content: String,
        numbered: String,
        byte_size: usize,
        modified_at: Option<SystemTime>,
        git: GitFileState,
    },
    Unavailable {
        path: PathBuf,
        reason: String,
        byte_size: Option<usize>,
        git: GitFileState,
    },
}

pub fn tree(root: &Path, expanded: &HashSet<PathBuf>) -> Result<Vec<TreeEntry>, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("Cannot inspect Worktree: {error}"))?;
    let statuses = git_status(&root);
    let mut entries = Vec::new();
    read_directory(&root, &root, 0, expanded, &statuses, &mut entries)?;
    Ok(entries)
}

fn read_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    expanded: &HashSet<PathBuf>,
    statuses: &BTreeMap<PathBuf, GitFileState>,
    output: &mut Vec<TreeEntry>,
) -> Result<(), String> {
    if output.len() >= MAX_TREE_ENTRIES {
        return Ok(());
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_directory = left.file_type().is_ok_and(|kind| kind.is_dir());
        let right_directory = right.file_type().is_ok_and(|kind| kind.is_dir());
        right_directory
            .cmp(&left_directory)
            .then_with(|| left.file_name().cmp(&right.file_name()))
    });
    for entry in entries.into_iter().take(MAX_DIRECTORY_ENTRIES) {
        if output.len() >= MAX_TREE_ENTRIES {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if excluded_name(&name) {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root).map(Path::to_path_buf) else {
            continue;
        };
        let git = statuses
            .get(&relative)
            .copied()
            .unwrap_or(GitFileState::Clean);
        if git == GitFileState::Ignored {
            continue;
        }
        let is_expanded = kind.is_dir() && expanded.contains(&relative);
        output.push(TreeEntry {
            path: relative.clone(),
            depth,
            directory: kind.is_dir(),
            expanded: is_expanded,
            git,
        });
        if is_expanded {
            read_directory(root, &path, depth + 1, expanded, statuses, output)?;
        }
    }
    Ok(())
}

pub fn inspect_file(root: &Path, relative: &Path) -> FilePreview {
    let git = git_status(root)
        .get(relative)
        .copied()
        .unwrap_or(GitFileState::Clean);
    let unavailable = |reason: String, byte_size| FilePreview::Unavailable {
        path: relative.to_path_buf(),
        reason,
        byte_size,
        git,
    };
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || relative
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(sensitive_name)
    {
        return unavailable(
            "File is outside the safe Worktree inspection boundary.".into(),
            None,
        );
    }
    let Ok(root) = root.canonicalize() else {
        return unavailable("Worktree is unavailable.".into(), None);
    };
    let Ok(path) = root.join(relative).canonicalize() else {
        return unavailable("File is unavailable.".into(), None);
    };
    if !path.starts_with(&root) || !path.is_file() {
        return unavailable("File is outside the registered Worktree.".into(), None);
    }
    let Ok(metadata) = path.metadata() else {
        return unavailable("File metadata is unavailable.".into(), None);
    };
    let size = metadata.len() as usize;
    if size > MAX_FILE_PREVIEW_BYTES {
        return unavailable(
            format!("File is too large for inline preview ({size} bytes)."),
            Some(size),
        );
    }
    let Ok(bytes) = fs::read(path) else {
        return unavailable("File cannot be read.".into(), Some(size));
    };
    let Ok(content) = String::from_utf8(bytes) else {
        return unavailable(
            "Binary or non-UTF-8 files cannot be previewed.".into(),
            Some(size),
        );
    };
    let numbered = content
        .lines()
        .enumerate()
        .map(|(index, line)| format!("{:>5}  {line}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    FilePreview::Text {
        path: relative.to_path_buf(),
        content,
        numbered,
        byte_size: size,
        modified_at: metadata.modified().ok(),
        git,
    }
}

pub fn search_lines(content: &str, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let query = query.to_lowercase();
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| line.to_lowercase().contains(&query).then_some(index + 1))
        .collect()
}

fn git_status(root: &Path) -> BTreeMap<PathBuf, GitFileState> {
    let Ok(output) = Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "status",
            "--porcelain=v1",
            "-z",
            "--ignored=matching",
        ])
        .output()
    else {
        return BTreeMap::new();
    };
    if !output.status.success() {
        return BTreeMap::new();
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            if record.len() < 4 {
                return None;
            }
            let status = std::str::from_utf8(&record[..2]).ok()?;
            let path = std::str::from_utf8(&record[3..]).ok()?;
            Some((PathBuf::from(path), parse_git_state(status)))
        })
        .collect()
}

fn parse_git_state(status: &str) -> GitFileState {
    match status {
        "??" => GitFileState::Untracked,
        "!!" => GitFileState::Ignored,
        value if value.contains('U') || matches!(value, "AA" | "DD") => GitFileState::Conflicted,
        value if value.contains('D') => GitFileState::Deleted,
        value if value.contains('A') => GitFileState::Added,
        value if value.contains('M') || value.contains('R') => GitFileState::Modified,
        _ => GitFileState::Clean,
    }
}

fn excluded_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    sensitive_name(name)
        || matches!(
            lower.as_str(),
            ".git" | "node_modules" | "target" | "build" | "dist" | ".next" | ".cache"
        )
}

fn sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn tree_is_lazy_bounded_and_sensitive_files_are_hidden() {
        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("README.md"), "project").unwrap();
        fs::write(root.path().join(".env"), "TOKEN=private").unwrap();
        fs::write(root.path().join("src/auth.rs"), "fn authenticate() {}").unwrap();
        let collapsed = tree(root.path(), &HashSet::new()).unwrap();
        assert!(collapsed.iter().any(|entry| entry.path == Path::new("src")));
        assert!(
            !collapsed
                .iter()
                .any(|entry| entry.path == Path::new("src/auth.rs"))
        );
        assert!(
            !collapsed
                .iter()
                .any(|entry| entry.path == Path::new(".env"))
        );
        let expanded = tree(root.path(), &HashSet::from([PathBuf::from("src")])).unwrap();
        assert!(
            expanded
                .iter()
                .any(|entry| entry.path == Path::new("src/auth.rs"))
        );
    }

    #[test]
    fn preview_is_bounded_numbered_and_searchable() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("auth.rs"), "fn auth() {}\nfn user() {}").unwrap();
        let FilePreview::Text {
            content, numbered, ..
        } = inspect_file(root.path(), Path::new("auth.rs"))
        else {
            panic!("expected text preview");
        };
        assert!(numbered.contains("    1  fn auth"));
        assert_eq!(search_lines(&content, "USER"), vec![2]);
        assert!(matches!(
            inspect_file(root.path(), Path::new("../auth.rs")),
            FilePreview::Unavailable { .. }
        ));
    }
}
