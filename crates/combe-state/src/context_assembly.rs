use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AiContinuityStore, ContextEdge, ContextFileDisposition, ContextGraph, ContextGraphProjector,
    ContextGraphStore, ContextNode, ContextNodeKind, GraphQueryOptions, ProviderRegistry,
    RecipientId, Result, StateError, Timestamp, WorkId, WorkStore,
};

pub const CONTEXT_PACKAGE_CONTRACT: &str = "COMBE_CONTEXT_PACKAGE";
pub const CONTEXT_PACKAGE_VERSION: u32 = 1;
pub const DEFAULT_CONTEXT_ITEMS: usize = 24;
pub const DEFAULT_CONTEXT_BYTES: usize = 64 * 1024;
pub const DEFAULT_CONTEXT_REFERENCE_BYTES: usize = 12 * 1024;
pub const DEFAULT_CONTEXT_CONTENT_BYTES: usize = 40 * 1024;
pub const DEFAULT_CONTEXT_FILE_BYTES: usize = 12 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRequest {
    pub work_id: WorkId,
    pub recipient_id: RecipientId,
    pub message: String,
    pub max_items: usize,
    pub max_bytes: usize,
    pub max_reference_bytes: usize,
    pub max_content_bytes: usize,
    pub max_file_bytes: usize,
}

impl ContextRequest {
    pub fn new(work_id: WorkId, recipient_id: RecipientId, message: impl Into<String>) -> Self {
        Self {
            work_id,
            recipient_id,
            message: message.into(),
            max_items: DEFAULT_CONTEXT_ITEMS,
            max_bytes: DEFAULT_CONTEXT_BYTES,
            max_reference_bytes: DEFAULT_CONTEXT_REFERENCE_BYTES,
            max_content_bytes: DEFAULT_CONTEXT_CONTENT_BYTES,
            max_file_bytes: DEFAULT_CONTEXT_FILE_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextItemProvenance {
    pub source_type: String,
    pub source_id: String,
    pub relation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: String,
    pub kind: ContextNodeKind,
    pub display: String,
    pub reference: String,
    pub provenance: Vec<ContextItemProvenance>,
    pub relevance: u32,
    pub reasons: Vec<String>,
    pub content: Option<String>,
    pub content_hash: Option<String>,
    pub byte_size: Option<usize>,
    pub content_status: ContextContentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextContentStatus {
    NotApplicable,
    Included,
    ReferenceOnly,
    Excluded,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPackage {
    pub contract: String,
    pub version: u32,
    pub work_id: WorkId,
    pub recipient_id: RecipientId,
    pub provider_profile_id: String,
    pub provider: String,
    pub model: Option<String>,
    pub worktree: String,
    pub message: String,
    pub items: Vec<ContextItem>,
    pub fingerprint: String,
    pub graph_fingerprint: String,
    pub work_revision: u64,
    pub generated_at: Timestamp,
    pub bytes: usize,
    pub truncated: bool,
    pub policy: ContextAssemblyPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextAssemblyPolicy {
    pub max_items: usize,
    pub max_bytes: usize,
    pub max_reference_bytes: usize,
    pub max_content_bytes: usize,
    pub max_file_bytes: usize,
}

impl ContextPackage {
    pub fn canonical_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn provider_text(&self) -> Result<String> {
        Ok(format!(
            "{CONTEXT_PACKAGE_CONTRACT} v{CONTEXT_PACKAGE_VERSION}\n{}\nEND_{CONTEXT_PACKAGE_CONTRACT}",
            self.canonical_json()?
        ))
    }
}

pub struct ContextAssembler;

impl ContextAssembler {
    pub fn assemble<S>(store: &S, request: &ContextRequest) -> Result<ContextPackage>
    where
        S: WorkStore + ProviderRegistry + AiContinuityStore + ContextGraphStore,
    {
        if request.message.trim().is_empty() {
            return Err(StateError::InvalidEntity("message cannot be empty".into()));
        }
        if request.max_items < 2 || request.max_bytes < 2048 {
            return Err(StateError::InvalidEntity(
                "context bounds must allow at least two items and 2048 bytes".into(),
            ));
        }
        if store.load_context_graph()?.is_none() {
            ContextGraphProjector::rebuild(store)?;
        }
        let health = ContextGraphProjector::verify(store)?;
        if !health.healthy {
            return Err(StateError::InvalidEntity(
                "context graph is stale or unhealthy; run `combe graph rebuild`".into(),
            ));
        }
        let graph = store
            .load_context_graph()?
            .ok_or_else(|| StateError::NotFound {
                entity_type: "context-graph".into(),
                id: "default".into(),
            })?;
        let context = store.context(&request.work_id)?;
        let recipient = store.get_recipient(&request.recipient_id)?;
        let profile = store.get_profile(&recipient.provider_profile_id)?;
        let root = format!("work:{}", request.work_id.0);
        let subgraph = graph.context(
            &root,
            &GraphQueryOptions {
                max_nodes: request.max_items.saturating_mul(4).max(32),
                max_edges: request.max_items.saturating_mul(8).max(64),
                max_bytes: request.max_bytes.saturating_mul(4),
                work_scope: Some(request.work_id.clone()),
                ..Default::default()
            },
        )?;
        let tokens = lexical_tokens(&request.message);
        let edge_index = edge_index(&subgraph.edges);
        let positions = subgraph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.as_str(), index))
            .collect::<BTreeMap<_, _>>();
        let mut candidates = subgraph
            .nodes
            .iter()
            .map(|node| item(node, edge_index.get(&node.id), &tokens, &context))
            .collect::<Vec<_>>();
        candidates.extend(repository_candidates(
            Path::new(&context.work.workspace_id),
            &tokens,
        ));
        let hints = store.context_file_hints(&request.work_id)?;
        let excluded = hints
            .iter()
            .filter(|hint| hint.disposition == ContextFileDisposition::Exclude)
            .map(|hint| hint.path.as_str())
            .collect::<BTreeSet<_>>();
        candidates.retain(|candidate| {
            candidate.kind != ContextNodeKind::File
                || !excluded.contains(candidate.reference.as_str())
        });
        for hint in hints
            .iter()
            .filter(|hint| hint.disposition == ContextFileDisposition::Include)
        {
            if let Some(candidate) = candidates.iter_mut().find(|candidate| {
                candidate.kind == ContextNodeKind::File && candidate.reference == hint.path
            }) {
                candidate.relevance = candidate.relevance.max(5000);
                candidate
                    .reasons
                    .push("Explicitly included by the Work".into());
            } else {
                candidates.push(ContextItem {
                    id: format!("file:{}:{}", context.work.workspace_id, hint.path),
                    kind: ContextNodeKind::File,
                    display: hint.path.clone(),
                    reference: hint.path.clone(),
                    provenance: vec![ContextItemProvenance {
                        source_type: "work_context_hint".into(),
                        source_id: request.work_id.0.clone(),
                        relation: "explicitly_included".into(),
                    }],
                    relevance: 5000,
                    reasons: vec!["Explicitly included by the Work".into()],
                    content: None,
                    content_hash: None,
                    byte_size: None,
                    content_status: ContextContentStatus::ReferenceOnly,
                });
            }
        }
        let mut seen = BTreeSet::new();
        candidates.retain(|candidate| {
            seen.insert(format!("{:?}:{}", candidate.kind, candidate.reference))
        });
        candidates.sort_by(|left, right| {
            kind_rank(left.kind)
                .cmp(&kind_rank(right.kind))
                .then_with(|| right.relevance.cmp(&left.relevance))
                .then_with(|| {
                    positions
                        .get(left.id.as_str())
                        .cmp(&positions.get(right.id.as_str()))
                })
                .then_with(|| left.id.cmp(&right.id))
        });
        let worktree = canonical_worktree(Path::new(&context.work.workspace_id))?;
        let mut content_bytes = 0usize;
        let mut reference_bytes = 0usize;
        for candidate in &mut candidates {
            reference_bytes = reference_bytes.saturating_add(candidate.reference.len());
            if reference_bytes > request.max_reference_bytes {
                candidate.reference = "[reference budget exceeded]".into();
                candidate.content_status = ContextContentStatus::ReferenceOnly;
                candidate
                    .reasons
                    .push("Reference byte budget exceeded".into());
                continue;
            }
            materialize_file(
                candidate,
                &worktree,
                request.max_file_bytes,
                request.max_content_bytes,
                &mut content_bytes,
            );
        }
        let mut items = Vec::new();
        let mut bytes = 0usize;
        let mut truncated = subgraph.truncated;
        for candidate in candidates {
            if items.len() >= request.max_items {
                truncated = true;
                break;
            }
            let size = serde_json::to_vec(&candidate)?.len();
            if bytes.saturating_add(size) > request.max_bytes {
                truncated = true;
                continue;
            }
            bytes += size;
            items.push(candidate);
        }
        let provider = format!("{:?}", profile.service).to_lowercase();
        let generated_at = graph.generated_at;
        let mut package = ContextPackage {
            contract: CONTEXT_PACKAGE_CONTRACT.into(),
            version: CONTEXT_PACKAGE_VERSION,
            work_id: request.work_id.clone(),
            recipient_id: request.recipient_id.clone(),
            provider_profile_id: profile.id.0,
            provider,
            model: profile.model,
            worktree: context.work.workspace_id,
            message: safe_text(request.message.trim()),
            items,
            fingerprint: String::new(),
            graph_fingerprint: graph.graph_fingerprint,
            work_revision: context.revision,
            generated_at,
            bytes,
            truncated,
            policy: ContextAssemblyPolicy {
                max_items: request.max_items,
                max_bytes: request.max_bytes,
                max_reference_bytes: request.max_reference_bytes,
                max_content_bytes: request.max_content_bytes,
                max_file_bytes: request.max_file_bytes,
            },
        };
        loop {
            package.fingerprint = package_fingerprint(&package)?;
            package.bytes = 0;
            package.bytes = serde_json::to_vec(&package)?.len();
            package.bytes = serde_json::to_vec(&package)?.len();
            if package.bytes <= request.max_bytes {
                break;
            }
            if package.items.len() <= 2 {
                return Err(StateError::InvalidEntity(
                    "context byte bound is too small for authoritative Work context".into(),
                ));
            }
            package.items.pop();
            package.truncated = true;
        }
        Ok(package)
    }
}

fn package_fingerprint(package: &ContextPackage) -> Result<String> {
    let value = serde_json::json!({
    "contract": package.contract,
    "version": package.version,
    "work_id": package.work_id,
    "recipient_id": package.recipient_id,
    "provider_profile_id": package.provider_profile_id,
    "provider": package.provider,
    "model": package.model,
    "worktree": package.worktree,
    "message": package.message,
    "items": package.items,
    "graph_fingerprint": package.graph_fingerprint,
    "work_revision": package.work_revision,
        "truncated": package.truncated,
        "assembly_policy": package.policy,
    });
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?)))
}

fn lexical_tokens(message: &str) -> BTreeSet<String> {
    message
        .split(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
        .map(str::to_lowercase)
        .filter(|token| token.len() > 2)
        .collect()
}

fn edge_index(edges: &[ContextEdge]) -> BTreeMap<String, Vec<&ContextEdge>> {
    let mut index = BTreeMap::<String, Vec<&ContextEdge>>::new();
    for edge in edges {
        index.entry(edge.from.clone()).or_default().push(edge);
        index.entry(edge.to.clone()).or_default().push(edge);
    }
    index
}

fn item(
    node: &ContextNode,
    edges: Option<&Vec<&ContextEdge>>,
    tokens: &BTreeSet<String>,
    context: &crate::WorkContext,
) -> ContextItem {
    let display = display(node, context);
    let searchable = format!(
        "{} {}",
        safe_text(&display),
        node.properties
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    )
    .to_lowercase();
    let matches = tokens
        .iter()
        .filter(|token| searchable.contains(token.as_str()))
        .count() as u32;
    let mut reasons = Vec::new();
    if node.kind == ContextNodeKind::Work {
        reasons.push("Authoritative Work state".into());
    } else if node.kind == ContextNodeKind::Worktree {
        reasons.push("Current Worktree".into());
    }
    if matches > 0 {
        reasons.push(format!("Matches {matches} message term(s)"));
    }
    let provenance = edges
        .into_iter()
        .flatten()
        .map(|edge| {
            reasons.push(format!("Graph relation: {}", edge.relation));
            ContextItemProvenance {
                source_type: edge.source.source_type.clone(),
                source_id: edge.source.source_id.clone(),
                relation: edge.relation.clone(),
            }
        })
        .collect();
    reasons.sort();
    reasons.dedup();
    ContextItem {
        id: node.id.clone(),
        kind: node.kind,
        display: safe_text(&display),
        reference: safe_reference(node),
        provenance,
        relevance: matches,
        reasons,
        content: None,
        content_hash: None,
        byte_size: None,
        content_status: if node.kind == ContextNodeKind::File {
            ContextContentStatus::ReferenceOnly
        } else {
            ContextContentStatus::NotApplicable
        },
    }
}

fn canonical_worktree(path: &Path) -> Result<PathBuf> {
    path.canonicalize().map_err(|error| {
        StateError::InvalidEntity(format!(
            "registered Worktree cannot be read: {} ({error})",
            path.display()
        ))
    })
}

fn repository_candidates(root: &Path, tokens: &BTreeSet<String>) -> Vec<ContextItem> {
    let Ok(root) = root.canonicalize() else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    collect_repository_paths(&root, &root, &mut paths, 0);
    let mut candidates = paths
        .into_iter()
        .filter_map(|path| {
            let relative = path
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let score = repository_score(&relative, tokens);
            (score > 0).then(|| ContextItem {
                id: format!("file:{}:{relative}", root.display()),
                kind: ContextNodeKind::File,
                display: relative.clone(),
                reference: relative.clone(),
                provenance: vec![ContextItemProvenance {
                    source_type: "worktree_observation".into(),
                    source_id: root.display().to_string(),
                    relation: "observed_in".into(),
                }],
                relevance: score,
                reasons: repository_reasons(&relative, tokens),
                content: None,
                content_hash: None,
                byte_size: None,
                content_status: ContextContentStatus::ReferenceOnly,
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .relevance
            .cmp(&left.relevance)
            .then_with(|| left.reference.cmp(&right.reference))
    });
    candidates.truncate(96);
    candidates
}

fn collect_repository_paths(root: &Path, directory: &Path, paths: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 || paths.len() >= 4096 {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries = entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if paths.len() >= 4096 {
            break;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if !excluded_directory(&name) {
                collect_repository_paths(root, &path, paths, depth + 1);
            }
        } else if file_type.is_file() && !sensitive_path(&name) {
            if path.starts_with(root) {
                paths.push(path);
            }
        }
    }
}

fn excluded_directory(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git"
            | "node_modules"
            | "target"
            | "build"
            | "dist"
            | ".next"
            | ".cache"
            | "vendor"
            | "coverage"
    )
}

fn sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.contains("keychain")
}

fn repository_score(path: &str, tokens: &BTreeSet<String>) -> u32 {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let orientation = match name {
        "readme" | "readme.md" | "readme.txt" => 1000,
        "package.json" | "cargo.toml" | "pyproject.toml" | "go.mod" | "go.sum"
        | "pnpm-lock.yaml" | "package-lock.json" | "yarn.lock" | "tsconfig.json" => 900,
        _ if name.starts_with("vite.config.") || name.starts_with("next.config.") => 850,
        "main.rs" | "lib.rs" | "main.ts" | "index.ts" | "main.js" | "index.js"
            if lower.starts_with("src/") || !lower.contains('/') =>
        {
            700
        }
        _ => 0,
    };
    let matches = lexical_path_matches(&lower, tokens) as u32;
    orientation
        .max(matches.saturating_mul(550) + u32::from(matches > 0 && lower.starts_with("src/")) * 75)
}

fn repository_reasons(path: &str, tokens: &BTreeSet<String>) -> Vec<String> {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let mut reasons = Vec::new();
    if name.starts_with("readme") {
        reasons.push("Repository overview".into());
    }
    if matches!(
        name,
        "package.json"
            | "cargo.toml"
            | "pyproject.toml"
            | "go.mod"
            | "go.sum"
            | "pnpm-lock.yaml"
            | "package-lock.json"
            | "yarn.lock"
            | "tsconfig.json"
    ) || name.starts_with("vite.config.")
        || name.starts_with("next.config.")
    {
        reasons.push("Project structure and dependencies".into());
    }
    let matches = lexical_path_matches(&lower, tokens);
    if matches > 0 {
        reasons.push(format!("Matches {matches} message term(s)"));
    }
    if reasons.is_empty() {
        reasons.push("Primary source entry point".into());
    }
    reasons
}

fn lexical_path_matches(path: &str, tokens: &BTreeSet<String>) -> usize {
    let components = path
        .split(|character: char| !character.is_alphanumeric())
        .filter(|component| component.len() >= 4)
        .collect::<Vec<_>>();
    tokens
        .iter()
        .filter(|token| {
            path.contains(token.as_str())
                || components.iter().any(|component| {
                    token.starts_with(component) || component.starts_with(token.as_str())
                })
        })
        .count()
}

fn materialize_file(
    item: &mut ContextItem,
    worktree: &Path,
    max_file_bytes: usize,
    max_content_bytes: usize,
    content_bytes: &mut usize,
) {
    if item.kind != ContextNodeKind::File {
        return;
    }
    if item.reference.starts_with('[') || sensitive_path(&item.reference) {
        item.reference = "[sensitive reference excluded]".into();
        item.content_status = ContextContentStatus::Excluded;
        item.reasons
            .push("Excluded because the path may contain credentials".into());
        return;
    }
    let relative = Path::new(&item.reference);
    let candidate = if relative.is_absolute() {
        relative.to_path_buf()
    } else {
        worktree.join(relative)
    };
    let Ok(candidate) = candidate.canonicalize() else {
        item.content_status = ContextContentStatus::Unavailable;
        item.reasons.push("File is unavailable".into());
        return;
    };
    if !candidate.starts_with(worktree) || !candidate.is_file() {
        item.content_status = ContextContentStatus::Excluded;
        item.reasons
            .push("Excluded because it is outside the registered Worktree".into());
        return;
    }
    let Ok(metadata) = candidate.metadata() else {
        item.content_status = ContextContentStatus::Unavailable;
        item.reasons.push("File metadata is unavailable".into());
        return;
    };
    let size = metadata.len() as usize;
    item.byte_size = Some(size);
    if size > max_file_bytes {
        item.content_status = ContextContentStatus::ReferenceOnly;
        item.reasons.push(format!(
            "Reference only — exceeds {max_file_bytes} byte file limit"
        ));
        return;
    }
    if content_bytes.saturating_add(size) > max_content_bytes {
        item.content_status = ContextContentStatus::ReferenceOnly;
        item.reasons
            .push("Reference only — content budget exhausted".into());
        return;
    }
    let Ok(bytes) = fs::read(&candidate) else {
        item.content_status = ContextContentStatus::Unavailable;
        item.reasons.push("File cannot be read".into());
        return;
    };
    let Ok(content) = String::from_utf8(bytes) else {
        item.content_status = ContextContentStatus::Excluded;
        item.reasons
            .push("Excluded because the file is not UTF-8 text".into());
        return;
    };
    if sensitive_content(&content) {
        item.content_status = ContextContentStatus::Excluded;
        item.reasons
            .push("Excluded because credential-shaped content was detected".into());
        return;
    }
    item.content_hash = Some(format!("{:x}", Sha256::digest(content.as_bytes())));
    item.content = Some(content);
    item.content_status = ContextContentStatus::Included;
    *content_bytes = content_bytes.saturating_add(size);
    item.reasons.push("Bounded file content included".into());
}

fn sensitive_content(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("-----begin private key-----")
        || lower.contains("-----begin rsa private key-----")
        || content
            .split(|character: char| {
                character.is_whitespace() || character == '"' || character == '\''
            })
            .any(|part| part.starts_with("sk-") && part.len() > 20)
}

fn display(node: &ContextNode, context: &crate::WorkContext) -> String {
    match node.kind {
        ContextNodeKind::Work => context
            .work
            .objective
            .clone()
            .unwrap_or_else(|| context.work.title.clone()),
        ContextNodeKind::Decision => context
            .decisions
            .iter()
            .find(|value| node.id.ends_with(&value.id.0))
            .map(|value| value.statement.clone())
            .unwrap_or_else(|| node.id.clone()),
        ContextNodeKind::Artifact => context
            .artifacts
            .iter()
            .find(|value| node.id.ends_with(&value.id.0))
            .and_then(|value| value.description.clone().or(value.path.clone()))
            .unwrap_or_else(|| node.id.clone()),
        _ => node
            .properties
            .get("title")
            .or_else(|| node.properties.get("path"))
            .or_else(|| node.properties.get("commit_id"))
            .or_else(|| node.properties.get("provider"))
            .cloned()
            .unwrap_or_else(|| node.id.clone()),
    }
}

fn safe_reference(node: &ContextNode) -> String {
    let reference = node
        .properties
        .get("path")
        .cloned()
        .unwrap_or_else(|| node.id.clone());
    let lower = reference.to_lowercase();
    if lower.contains(".env")
        || lower.contains("credential")
        || lower.contains("keychain")
        || lower.contains("cookie")
        || lower.contains("secret")
    {
        "[sensitive reference excluded]".into()
    } else {
        reference
    }
}

fn safe_text(value: &str) -> String {
    let lower = value.to_lowercase();
    if lower.contains("authorization: bearer")
        || value
            .split_whitespace()
            .any(|part| part.starts_with("sk-") && part.len() > 12)
    {
        "[sensitive text excluded]".into()
    } else {
        value.into()
    }
}

fn kind_rank(kind: ContextNodeKind) -> u8 {
    match kind {
        ContextNodeKind::Work => 0,
        ContextNodeKind::Worktree => 1,
        ContextNodeKind::Decision => 2,
        ContextNodeKind::File => 3,
        ContextNodeKind::Commit => 4,
        ContextNodeKind::Conversation => 5,
        ContextNodeKind::ProviderExchange => 6,
        ContextNodeKind::Artifact => 7,
        ContextNodeKind::Message => 8,
        ContextNodeKind::ContextSnapshot => 9,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str) -> ContextNode {
        ContextNode {
            id: format!("file:{path}"),
            kind: ContextNodeKind::File,
            properties: BTreeMap::from([("path".into(), path.into())]),
        }
    }

    #[test]
    fn lexical_relevance_is_deterministic() {
        let tokens = lexical_tokens("Fix the password reset email template");
        assert_eq!(
            tokens,
            lexical_tokens("Fix the password reset email template")
        );
        assert!(tokens.contains("password"));
        assert!(tokens.contains("template"));
    }

    #[test]
    fn sensitive_references_are_excluded() {
        assert_eq!(
            safe_reference(&node(".env")),
            "[sensitive reference excluded]"
        );
        assert_eq!(safe_reference(&node("src/auth.rs")), "src/auth.rs");
        assert_eq!(
            safe_text("Authorization: Bearer private"),
            "[sensitive text excluded]"
        );
    }
}
