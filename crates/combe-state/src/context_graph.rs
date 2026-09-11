use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AiContextEventData, AiContinuityStore, ArtifactKind, Result, StateError, Timestamp, WorkId,
    WorkStore,
};

pub const CONTEXT_GRAPH_CONTRACT: &str = "COMBE_CONTEXT_GRAPH";
pub const CONTEXT_GRAPH_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextNodeKind {
    Work,
    Conversation,
    Message,
    Decision,
    Artifact,
    File,
    Commit,
    ProviderExchange,
    Worktree,
    ContextSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContextNode {
    pub id: String,
    pub kind: ContextNodeKind,
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipConfidence {
    Explicit,
    Derived,
    Observed,
    Inferred,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContextEdgeSource {
    pub source_type: String,
    pub source_id: String,
    pub confidence: RelationshipConfidence,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContextEdge {
    pub id: String,
    pub from: String,
    pub relation: String,
    pub to: String,
    pub source: ContextEdgeSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextGraphProjection {
    pub contract: String,
    pub version: u32,
    pub graph_id: String,
    pub projection_revision: String,
    pub generated_at: Timestamp,
    pub source_revision: String,
    pub graph_fingerprint: String,
    pub nodes: Vec<ContextNode>,
    pub edges: Vec<ContextEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub file_identity: String,
    pub path: String,
    pub previous_path: Option<String>,
    pub confidence: RelationshipConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeObservation {
    pub id: String,
    pub repository_id: String,
    pub worktree_id: String,
    pub work_id: Option<WorkId>,
    pub commit_id: String,
    pub parent_commit_ids: Vec<String>,
    pub branch: Option<String>,
    pub timestamp: Timestamp,
    pub files: Vec<FileChange>,
}

pub trait ContextGraphStore {
    fn load_context_graph(&self) -> Result<Option<ContextGraphProjection>>;
    fn save_context_graph(&self, projection: &ContextGraphProjection) -> Result<()>;
    fn record_change_observation(&self, observation: ChangeObservation) -> Result<()>;
    fn change_observations(&self) -> Result<Vec<ChangeObservation>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphQueryOptions {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_bytes: usize,
    pub max_messages: usize,
    pub max_commits: usize,
    pub include: BTreeSet<ContextNodeKind>,
    pub exclude: BTreeSet<ContextNodeKind>,
    pub work_scope: Option<WorkId>,
}

impl Default for GraphQueryOptions {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_nodes: 100,
            max_edges: 250,
            max_bytes: 256 * 1024,
            max_messages: 50,
            max_commits: 50,
            include: BTreeSet::new(),
            exclude: BTreeSet::new(),
            work_scope: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphRelation {
    pub edge: ContextEdge,
    pub node: ContextNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSubgraph {
    pub root: String,
    pub nodes: Vec<ContextNode>,
    pub edges: Vec<ContextEdge>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphHealth {
    pub contract: String,
    pub version: u32,
    pub projection_fingerprint: Option<String>,
    pub expected_fingerprint: String,
    pub healthy: bool,
    pub stale: bool,
    pub nodes: usize,
    pub edges: usize,
    pub missing_nodes: usize,
    pub missing_edges: usize,
    pub orphaned_edges: usize,
    pub duplicate_relationships: usize,
    pub stale_sources: usize,
    pub source_revision: String,
}

pub trait ContextGraph {
    fn related(&self, node: &str, options: &GraphQueryOptions) -> Result<Vec<GraphRelation>>;
    fn traverse(&self, node: &str, options: &GraphQueryOptions) -> Result<GraphSubgraph>;
    fn context(&self, node: &str, options: &GraphQueryOptions) -> Result<GraphSubgraph>;
    fn fingerprint(&self) -> &str;
}

impl ContextGraph for ContextGraphProjection {
    fn related(&self, node: &str, options: &GraphQueryOptions) -> Result<Vec<GraphRelation>> {
        let graph = self.traverse(
            node,
            &GraphQueryOptions {
                max_depth: 1,
                ..options.clone()
            },
        )?;
        let nodes = graph
            .nodes
            .into_iter()
            .map(|value| (value.id.clone(), value))
            .collect::<BTreeMap<_, _>>();
        Ok(graph
            .edges
            .into_iter()
            .filter_map(|edge| {
                let related = if edge.from == node {
                    &edge.to
                } else {
                    &edge.from
                };
                nodes
                    .get(related)
                    .cloned()
                    .map(|node| GraphRelation { edge, node })
            })
            .collect())
    }

    fn traverse(&self, root: &str, options: &GraphQueryOptions) -> Result<GraphSubgraph> {
        let all_nodes = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        if !all_nodes.contains_key(root) {
            return Err(StateError::NotFound {
                entity_type: "context-node".into(),
                id: root.into(),
            });
        }
        let allowed = |node: &ContextNode| {
            !options.exclude.contains(&node.kind)
                && (options.include.is_empty() || options.include.contains(&node.kind))
                && options.work_scope.as_ref().is_none_or(|scope| {
                    node.id == format!("work:{}", scope.0)
                        || node
                            .properties
                            .get("work_id")
                            .is_none_or(|id| id == &scope.0)
                })
        };
        let mut queue = VecDeque::from([(root.to_string(), 0usize)]);
        let mut visited = BTreeSet::new();
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut bytes = 0usize;
        let mut messages = 0usize;
        let mut commits = 0usize;
        let mut truncated = false;
        while let Some((id, depth)) = queue.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            let Some(node) = all_nodes.get(id.as_str()) else {
                continue;
            };
            if id != root && !allowed(node) {
                continue;
            }
            if node.kind == ContextNodeKind::Message && messages >= options.max_messages
                || node.kind == ContextNodeKind::Commit && commits >= options.max_commits
            {
                truncated = true;
                continue;
            }
            let size = serde_json::to_vec(node)?.len();
            if nodes.len() >= options.max_nodes || bytes.saturating_add(size) > options.max_bytes {
                truncated = true;
                break;
            }
            bytes += size;
            messages += usize::from(node.kind == ContextNodeKind::Message);
            commits += usize::from(node.kind == ContextNodeKind::Commit);
            nodes.push((*node).clone());
            if depth >= options.max_depth {
                continue;
            }
            let mut adjacent = self
                .edges
                .iter()
                .filter(|edge| edge.from == id || edge.to == id)
                .collect::<Vec<_>>();
            adjacent.sort_by(|left, right| {
                let left_id = if left.from == id {
                    &left.to
                } else {
                    &left.from
                };
                let right_id = if right.from == id {
                    &right.to
                } else {
                    &right.from
                };
                let left_kind = all_nodes.get(left_id.as_str()).map(|node| node.kind);
                let right_kind = all_nodes.get(right_id.as_str()).map(|node| node.kind);
                confidence_rank(left.source.confidence)
                    .cmp(&confidence_rank(right.source.confidence))
                    .then_with(|| node_rank(left_kind).cmp(&node_rank(right_kind)))
                    .then_with(|| left.id.cmp(&right.id))
            });
            for edge in adjacent {
                if edges
                    .iter()
                    .any(|present: &ContextEdge| present.id == edge.id)
                {
                    continue;
                }
                let next = if edge.from == id {
                    &edge.to
                } else {
                    &edge.from
                };
                let Some(next_node) = all_nodes.get(next.as_str()) else {
                    continue;
                };
                if !allowed(next_node) {
                    continue;
                }
                let size = serde_json::to_vec(edge)?.len();
                if edges.len() >= options.max_edges
                    || bytes.saturating_add(size) > options.max_bytes
                {
                    truncated = true;
                    break;
                }
                bytes += size;
                edges.push(edge.clone());
                queue.push_back((next.clone(), depth + 1));
            }
        }
        nodes.sort();
        let selected = nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<BTreeSet<_>>();
        edges.retain(|edge| {
            selected.contains(edge.from.as_str()) && selected.contains(edge.to.as_str())
        });
        edges.sort();
        Ok(GraphSubgraph {
            root: root.into(),
            nodes,
            edges,
            truncated,
        })
    }

    fn context(&self, node: &str, options: &GraphQueryOptions) -> Result<GraphSubgraph> {
        self.traverse(node, options)
    }

    fn fingerprint(&self) -> &str {
        &self.graph_fingerprint
    }
}

pub struct ContextGraphProjector;

impl ContextGraphProjector {
    pub fn build<S>(store: &S) -> Result<ContextGraphProjection>
    where
        S: WorkStore + AiContinuityStore + ContextGraphStore,
    {
        let mut builder = GraphBuilder::default();
        for work in store.list_works(None)? {
            let work_id = node_id(ContextNodeKind::Work, &work.id.0);
            builder.node(
                ContextNodeKind::Work,
                &work.id.0,
                [
                    ("work_id", work.id.0.as_str()),
                    ("worktree_id", work.workspace_id.as_str()),
                    ("status", &format!("{:?}", work.status).to_lowercase()),
                    ("title", work.title.as_str()),
                ],
            );
            let worktree_id = builder.node(
                ContextNodeKind::Worktree,
                &work.workspace_id,
                [
                    ("worktree_id", work.workspace_id.as_str()),
                    ("path", work.workspace_id.as_str()),
                    (
                        "available",
                        if std::path::Path::new(&work.workspace_id).exists() {
                            "true"
                        } else {
                            "false"
                        },
                    ),
                ],
            );
            builder.edge(
                &work_id,
                "executed_in",
                &worktree_id,
                "work",
                &work.id.0,
                RelationshipConfidence::Explicit,
            );
            let context = store.context(&work.id)?;
            for conversation in context.conversations {
                let id = builder.node(
                    ContextNodeKind::Conversation,
                    &conversation.conversation.id,
                    [
                        ("conversation_id", conversation.conversation.id.as_str()),
                        ("work_id", work.id.0.as_str()),
                        (
                            "title",
                            conversation
                                .label
                                .as_deref()
                                .or(conversation.conversation.title.as_deref())
                                .unwrap_or(""),
                        ),
                    ],
                );
                builder.edge(
                    &id,
                    "belongs_to",
                    &work_id,
                    "work_conversation",
                    &conversation.id.0,
                    RelationshipConfidence::Explicit,
                );
            }
            for decision in context.decisions {
                let id = builder.node(
                    ContextNodeKind::Decision,
                    &decision.id.0,
                    [
                        ("decision_id", decision.id.0.as_str()),
                        ("work_id", work.id.0.as_str()),
                        ("status", "recorded"),
                        ("created_at", &decision.created_at.to_rfc3339()),
                    ],
                );
                builder.edge(
                    &work_id,
                    "produced",
                    &id,
                    "decision",
                    &decision.id.0,
                    RelationshipConfidence::Explicit,
                );
            }
            for artifact in context.artifacts {
                let id = builder.node(
                    ContextNodeKind::Artifact,
                    &artifact.id.0,
                    [
                        ("artifact_id", artifact.id.0.as_str()),
                        ("work_id", work.id.0.as_str()),
                        ("kind", &format!("{:?}", artifact.kind).to_lowercase()),
                        ("path", artifact.path.as_deref().unwrap_or("")),
                    ],
                );
                builder.edge(
                    &id,
                    "produced_by",
                    &work_id,
                    "artifact",
                    &artifact.id.0,
                    RelationshipConfidence::Explicit,
                );
                if artifact.kind == ArtifactKind::File
                    && let Some(path) = artifact.path.as_deref()
                {
                    let identity = format!("path:{}:{path}", work.workspace_id);
                    let file = builder.node(
                        ContextNodeKind::File,
                        &format!("{}:{path}", work.workspace_id),
                        [
                            ("file_identity", identity.as_str()),
                            ("path", path),
                            ("work_id", work.id.0.as_str()),
                            ("identity_confidence", "path"),
                        ],
                    );
                    builder.edge(
                        &work_id,
                        "associated_with",
                        &file,
                        "artifact",
                        &artifact.id.0,
                        RelationshipConfidence::Explicit,
                    );
                    builder.edge(
                        &id,
                        "references",
                        &file,
                        "artifact",
                        &artifact.id.0,
                        RelationshipConfidence::Explicit,
                    );
                }
                if artifact.kind == ArtifactKind::Commit
                    && let Some(commit) = artifact.description.as_deref()
                {
                    let commit_node = builder.node(
                        ContextNodeKind::Commit,
                        commit,
                        [("commit_id", commit), ("work_id", work.id.0.as_str())],
                    );
                    builder.edge(
                        &commit_node,
                        "implemented_by",
                        &work_id,
                        "artifact",
                        &artifact.id.0,
                        RelationshipConfidence::Derived,
                    );
                    builder.edge(
                        &id,
                        "references",
                        &commit_node,
                        "artifact",
                        &artifact.id.0,
                        RelationshipConfidence::Explicit,
                    );
                }
            }
        }
        for conversation_id in store.ai_conversation_ids()? {
            let events = store.ai_events(&conversation_id)?;
            let conversation = builder.node(
                ContextNodeKind::Conversation,
                &conversation_id,
                [
                    ("conversation_id", conversation_id.as_str()),
                    ("provider_neutral", "true"),
                ],
            );
            for event in events {
                match event.event {
                    AiContextEventData::MessageCreated { message } => {
                        let message_node = builder.node(
                            ContextNodeKind::Message,
                            &message.id,
                            [
                                ("message_id", message.id.as_str()),
                                ("conversation_id", conversation_id.as_str()),
                                ("role", message.role.as_str()),
                                ("created_at", &message.created_at.to_rfc3339()),
                            ],
                        );
                        builder.edge(
                            &conversation,
                            "contains",
                            &message_node,
                            "canonical_event",
                            &event.id,
                            RelationshipConfidence::Explicit,
                        );
                    }
                    AiContextEventData::WorkLinked { work_id } => {
                        builder.edge(
                            &conversation,
                            "belongs_to",
                            &node_id(ContextNodeKind::Work, &work_id.0),
                            "canonical_event",
                            &event.id,
                            RelationshipConfidence::Explicit,
                        );
                    }
                    AiContextEventData::DecisionCreated { decision } => {
                        let decision_node = builder.node(
                            ContextNodeKind::Decision,
                            &decision.id,
                            [
                                ("decision_id", decision.id.as_str()),
                                ("status", "recorded"),
                            ],
                        );
                        builder.edge(
                            &conversation,
                            "produced",
                            &decision_node,
                            "canonical_event",
                            &event.id,
                            RelationshipConfidence::Explicit,
                        );
                    }
                    AiContextEventData::DecisionRevised { decision_id, .. } => {
                        let decision_node = builder.node(
                            ContextNodeKind::Decision,
                            &decision_id,
                            [("decision_id", decision_id.as_str()), ("status", "revised")],
                        );
                        builder.edge(
                            &conversation,
                            "produced",
                            &decision_node,
                            "canonical_event",
                            &event.id,
                            RelationshipConfidence::Explicit,
                        );
                    }
                    AiContextEventData::ArtifactAttached { artifact } => {
                        let artifact_node = builder.node(
                            ContextNodeKind::Artifact,
                            &artifact.id,
                            [
                                ("artifact_id", artifact.id.as_str()),
                                ("kind", artifact.kind.as_str()),
                            ],
                        );
                        builder.edge(
                            &conversation,
                            "produced",
                            &artifact_node,
                            "canonical_event",
                            &event.id,
                            RelationshipConfidence::Explicit,
                        );
                    }
                    _ => {}
                }
            }
            for exchange in store.ai_exchanges(&conversation_id)? {
                let exchange_node = builder.node(
                    ContextNodeKind::ProviderExchange,
                    &exchange.id,
                    [
                        ("exchange_id", exchange.id.as_str()),
                        ("provider", exchange.provider.as_str()),
                        ("model", exchange.model.as_deref().unwrap_or("")),
                        ("conversation_id", conversation_id.as_str()),
                        ("context_fingerprint", exchange.context_fingerprint.as_str()),
                    ],
                );
                let snapshot = builder.node(
                    ContextNodeKind::ContextSnapshot,
                    &exchange.context_fingerprint,
                    [
                        ("context_fingerprint", exchange.context_fingerprint.as_str()),
                        ("contract", "COMBE_AI_CONTEXT"),
                        ("version", "1"),
                    ],
                );
                builder.edge(
                    &exchange_node,
                    "generated_from",
                    &conversation,
                    "provider_exchange",
                    &exchange.id,
                    RelationshipConfidence::Explicit,
                );
                builder.edge(
                    &exchange_node,
                    "generated_from",
                    &snapshot,
                    "provider_exchange",
                    &exchange.id,
                    RelationshipConfidence::Explicit,
                );
                if let Some(work_id) = exchange.work_id {
                    builder.edge(
                        &exchange_node,
                        "associated_with",
                        &node_id(ContextNodeKind::Work, &work_id.0),
                        "provider_exchange",
                        &exchange.id,
                        RelationshipConfidence::Explicit,
                    );
                }
            }
        }
        for observation in store.change_observations()? {
            builder.change(observation);
        }
        Ok(builder.finish())
    }

    pub fn rebuild<S>(store: &S) -> Result<ContextGraphProjection>
    where
        S: WorkStore + AiContinuityStore + ContextGraphStore,
    {
        let projection = Self::build(store)?;
        store.save_context_graph(&projection)?;
        Ok(projection)
    }

    pub fn verify<S>(store: &S) -> Result<GraphHealth>
    where
        S: WorkStore + AiContinuityStore + ContextGraphStore,
    {
        let expected = Self::build(store)?;
        let actual = store.load_context_graph()?;
        let actual_nodes = actual
            .as_ref()
            .map_or(&[][..], |value| value.nodes.as_slice());
        let actual_edges = actual
            .as_ref()
            .map_or(&[][..], |value| value.edges.as_slice());
        let expected_node_ids = expected
            .nodes
            .iter()
            .map(|value| &value.id)
            .collect::<BTreeSet<_>>();
        let actual_node_ids = actual_nodes
            .iter()
            .map(|value| &value.id)
            .collect::<BTreeSet<_>>();
        let expected_edge_ids = expected
            .edges
            .iter()
            .map(|value| &value.id)
            .collect::<BTreeSet<_>>();
        let actual_edge_ids = actual_edges
            .iter()
            .map(|value| &value.id)
            .collect::<BTreeSet<_>>();
        let duplicate_relationships = actual_edges.len().saturating_sub(actual_edge_ids.len());
        let orphaned_edges = actual_edges
            .iter()
            .filter(|edge| {
                !actual_node_ids.contains(&edge.from) || !actual_node_ids.contains(&edge.to)
            })
            .count();
        let missing_nodes = expected_node_ids.difference(&actual_node_ids).count();
        let missing_edges = expected_edge_ids.difference(&actual_edge_ids).count();
        let stale = actual
            .as_ref()
            .is_none_or(|value| value.graph_fingerprint != expected.graph_fingerprint);
        Ok(GraphHealth {
            contract: CONTEXT_GRAPH_CONTRACT.into(),
            version: CONTEXT_GRAPH_VERSION,
            projection_fingerprint: actual.as_ref().map(|value| value.graph_fingerprint.clone()),
            expected_fingerprint: expected.graph_fingerprint.clone(),
            healthy: !stale && orphaned_edges == 0 && duplicate_relationships == 0,
            stale,
            nodes: actual_nodes.len(),
            edges: actual_edges.len(),
            missing_nodes,
            missing_edges,
            orphaned_edges,
            duplicate_relationships,
            stale_sources: usize::from(stale),
            source_revision: expected.source_revision,
        })
    }
}

#[derive(Default)]
struct GraphBuilder {
    nodes: BTreeMap<String, ContextNode>,
    edges: BTreeMap<String, ContextEdge>,
}

impl GraphBuilder {
    fn node<'a>(
        &mut self,
        kind: ContextNodeKind,
        identity: &str,
        properties: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> String {
        let id = node_id(kind, identity);
        let properties: BTreeMap<String, String> = properties
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self.nodes
            .entry(id.clone())
            .and_modify(|node| node.properties.extend(properties.clone()))
            .or_insert(ContextNode {
                id: id.clone(),
                kind,
                properties,
            });
        id
    }

    fn edge(
        &mut self,
        from: &str,
        relation: &str,
        to: &str,
        source_type: &str,
        source_id: &str,
        confidence: RelationshipConfidence,
    ) {
        let id = digest(&(source_type, source_id, from, relation, to));
        self.edges.insert(
            id.clone(),
            ContextEdge {
                id,
                from: from.into(),
                relation: relation.into(),
                to: to.into(),
                source: ContextEdgeSource {
                    source_type: source_type.into(),
                    source_id: source_id.into(),
                    confidence,
                    reason: None,
                },
            },
        );
    }

    fn change(&mut self, mut observation: ChangeObservation) {
        observation.parent_commit_ids.sort();
        observation.files.sort_by(|left, right| {
            left.file_identity
                .cmp(&right.file_identity)
                .then_with(|| left.path.cmp(&right.path))
        });
        let commit = self.node(
            ContextNodeKind::Commit,
            &format!("{}:{}", observation.repository_id, observation.commit_id),
            [
                ("commit_id", observation.commit_id.as_str()),
                ("repository_id", observation.repository_id.as_str()),
                ("branch", observation.branch.as_deref().unwrap_or("")),
                ("timestamp", &observation.timestamp.to_rfc3339()),
            ],
        );
        let worktree = self.node(
            ContextNodeKind::Worktree,
            &observation.worktree_id,
            [
                ("worktree_id", observation.worktree_id.as_str()),
                ("repository_id", observation.repository_id.as_str()),
            ],
        );
        self.edge(
            &commit,
            "observed_in",
            &worktree,
            "git_observation",
            &observation.id,
            RelationshipConfidence::Observed,
        );
        if let Some(work_id) = &observation.work_id {
            self.edge(
                &commit,
                "implemented_by",
                &node_id(ContextNodeKind::Work, &work_id.0),
                "git_observation",
                &observation.id,
                RelationshipConfidence::Observed,
            );
        }
        for parent in &observation.parent_commit_ids {
            let parent = self.node(
                ContextNodeKind::Commit,
                &format!("{}:{parent}", observation.repository_id),
                [
                    ("commit_id", parent.as_str()),
                    ("repository_id", observation.repository_id.as_str()),
                ],
            );
            self.edge(
                &commit,
                "parent_commit",
                &parent,
                "git_observation",
                &observation.id,
                RelationshipConfidence::Observed,
            );
        }
        for file in &observation.files {
            let file_node = self.node(
                ContextNodeKind::File,
                &format!("{}:{}", observation.repository_id, file.file_identity),
                [
                    ("file_identity", file.file_identity.as_str()),
                    ("repository_id", observation.repository_id.as_str()),
                    ("path", file.path.as_str()),
                    ("path_at_revision", file.path.as_str()),
                    ("previous_path", file.previous_path.as_deref().unwrap_or("")),
                    (
                        "identity_confidence",
                        &format!("{:?}", file.confidence).to_lowercase(),
                    ),
                ],
            );
            self.edge(
                &commit,
                "changed",
                &file_node,
                "git_observation",
                &observation.id,
                file.confidence,
            );
            if let Some(work_id) = &observation.work_id {
                self.edge(
                    &node_id(ContextNodeKind::Work, &work_id.0),
                    "associated_with",
                    &file_node,
                    "git_observation",
                    &observation.id,
                    file.confidence,
                );
            }
        }
    }

    fn finish(self) -> ContextGraphProjection {
        let nodes = self.nodes.into_values().collect::<Vec<_>>();
        let edges = self.edges.into_values().collect::<Vec<_>>();
        let source_revision = digest(&(&nodes, &edges));
        let graph_fingerprint = digest(&(&nodes, &edges, &source_revision));
        ContextGraphProjection {
            contract: CONTEXT_GRAPH_CONTRACT.into(),
            version: CONTEXT_GRAPH_VERSION,
            graph_id: "combe-context".into(),
            projection_revision: graph_fingerprint.clone(),
            generated_at: Utc::now(),
            source_revision,
            graph_fingerprint,
            nodes,
            edges,
        }
    }
}

fn node_id(kind: ContextNodeKind, identity: &str) -> String {
    format!(
        "{}:{identity}",
        serde_json::to_string(&kind).unwrap().trim_matches('"')
    )
}

fn confidence_rank(confidence: RelationshipConfidence) -> u8 {
    match confidence {
        RelationshipConfidence::Explicit => 0,
        RelationshipConfidence::Derived => 1,
        RelationshipConfidence::Observed => 2,
        RelationshipConfidence::Inferred => 3,
    }
}

fn node_rank(kind: Option<ContextNodeKind>) -> u8 {
    match kind {
        Some(ContextNodeKind::Work) => 0,
        Some(ContextNodeKind::Decision) => 1,
        Some(ContextNodeKind::File) => 2,
        Some(ContextNodeKind::Commit) => 3,
        Some(ContextNodeKind::Conversation) => 4,
        Some(ContextNodeKind::Artifact) => 5,
        Some(ContextNodeKind::ProviderExchange) => 6,
        Some(ContextNodeKind::ContextSnapshot) => 7,
        Some(ContextNodeKind::Message) => 8,
        Some(ContextNodeKind::Worktree) => 9,
        None => 10,
    }
}

fn digest(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("serializable context graph value");
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{
        AiContextEventData, AiParticipant, ArtifactId, Participant, ParticipantKind, Work,
        WorkArtifact, WorkDecision, WorkStore, new_ai_event,
    };

    fn fixture() -> (TempDir, crate::FeltDbWorkStore, Work, Participant) {
        let directory = TempDir::new().unwrap();
        let store = crate::FeltDbWorkStore::open(directory.path().join("graph.db")).unwrap();
        let work = Work::new(
            directory.path().to_string_lossy().into_owned(),
            "Context graph".into(),
            None,
        );
        let human = Participant::new(work.id.clone(), ParticipantKind::Human, "Human".into());
        store
            .create_work(work.clone(), vec![human.clone()])
            .unwrap();
        store
            .record_decision(
                WorkDecision {
                    id: crate::DecisionId("decision-1".into()),
                    work_id: work.id.clone(),
                    statement: "Keep file identity across renames".into(),
                    rationale: None,
                    decided_by: human.id.clone(),
                    created_at: Utc::now(),
                    proposal_id: None,
                    review_id: None,
                },
                None,
            )
            .unwrap();
        store
            .add_artifact(WorkArtifact {
                id: ArtifactId("artifact-1".into()),
                work_id: work.id.clone(),
                kind: ArtifactKind::File,
                path: Some("src/old.rs".into()),
                description: None,
                created_by: human.id.clone(),
                created_at: Utc::now(),
            })
            .unwrap();
        (directory, store, work, human)
    }

    #[test]
    fn projection_is_deterministic_idempotent_and_rebuildable() {
        let (_directory, store, work, _human) = fixture();
        let timestamp = Utc::now();
        let observation = ChangeObservation {
            id: "change-1".into(),
            repository_id: "repo-1".into(),
            worktree_id: work.workspace_id.clone(),
            work_id: Some(work.id.clone()),
            commit_id: "abc123".into(),
            parent_commit_ids: vec!["parent".into()],
            branch: Some("main".into()),
            timestamp,
            files: vec![FileChange {
                file_identity: "blob-lineage-1".into(),
                path: "src/new.rs".into(),
                previous_path: Some("src/old.rs".into()),
                confidence: RelationshipConfidence::Observed,
            }],
        };
        store
            .record_change_observation(observation.clone())
            .unwrap();
        store.record_change_observation(observation).unwrap();
        let first = ContextGraphProjector::build(&store).unwrap();
        let second = ContextGraphProjector::build(&store).unwrap();
        assert_eq!(first.graph_fingerprint, second.graph_fingerprint);
        assert_eq!(first.nodes, second.nodes);
        assert_eq!(first.edges, second.edges);
        assert_eq!(
            first
                .nodes
                .iter()
                .filter(|node| {
                    node.kind == ContextNodeKind::File
                        && node.properties.get("file_identity")
                            == Some(&"blob-lineage-1".to_string())
                })
                .count(),
            1
        );
        ContextGraphProjector::rebuild(&store).unwrap();
        assert!(ContextGraphProjector::verify(&store).unwrap().healthy);
        store
            .record_change_observation(ChangeObservation {
                id: "change-incremental".into(),
                repository_id: "repo-1".into(),
                worktree_id: work.workspace_id,
                work_id: Some(work.id),
                commit_id: "def456".into(),
                parent_commit_ids: vec!["abc123".into()],
                branch: Some("main".into()),
                timestamp: timestamp + chrono::Duration::seconds(1),
                files: Vec::new(),
            })
            .unwrap();
        assert!(ContextGraphProjector::verify(&store).unwrap().healthy);
    }

    #[test]
    fn traversal_is_bounded_and_work_scoped() {
        let (_directory, store, work, _human) = fixture();
        let unrelated = Work::new("same-repository".into(), "Unrelated".into(), None);
        store
            .create_work(
                unrelated.clone(),
                vec![Participant::new(
                    unrelated.id.clone(),
                    ParticipantKind::Human,
                    "Other".into(),
                )],
            )
            .unwrap();
        let projection = ContextGraphProjector::build(&store).unwrap();
        let graph = projection
            .traverse(
                &format!("work:{}", work.id.0),
                &GraphQueryOptions {
                    max_depth: 4,
                    max_nodes: 2,
                    max_edges: 1,
                    max_bytes: 4096,
                    work_scope: Some(work.id.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(graph.nodes.len() <= 2);
        assert!(graph.edges.len() <= 1);
        assert!(graph.truncated);
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| !node.id.contains(&unrelated.id.0))
        );
    }

    #[test]
    fn graph_relationship_changes_change_the_fingerprint() {
        let (_directory, store, work, _human) = fixture();
        let before = ContextGraphProjector::build(&store).unwrap();
        store
            .record_change_observation(ChangeObservation {
                id: "change-2".into(),
                repository_id: "repo".into(),
                worktree_id: work.workspace_id.clone(),
                work_id: Some(work.id),
                commit_id: "commit".into(),
                parent_commit_ids: Vec::new(),
                branch: None,
                timestamp: Utc::now(),
                files: Vec::new(),
            })
            .unwrap();
        let after = ContextGraphProjector::build(&store).unwrap();
        assert_ne!(before.graph_fingerprint, after.graph_fingerprint);
    }

    #[test]
    fn selected_graph_context_changes_ai_context_fingerprint() {
        let (_directory, store, work, _human) = fixture();
        let conversation_id = "conversation-graph";
        store
            .append_ai_event(new_ai_event(
                conversation_id,
                AiContextEventData::ConversationCreated {
                    participants: vec![AiParticipant {
                        id: "human".into(),
                        name: "Human".into(),
                        role: "human".into(),
                    }],
                    summary: "Graph continuity".into(),
                },
            ))
            .unwrap();
        store
            .append_ai_event(new_ai_event(
                conversation_id,
                AiContextEventData::WorkLinked {
                    work_id: work.id.clone(),
                },
            ))
            .unwrap();
        ContextGraphProjector::rebuild(&store).unwrap();
        let before = crate::resolve_conversation_context_with_graph(
            &store,
            conversation_id,
            crate::ResolveConversationOptions::default(),
        )
        .unwrap();
        store
            .record_change_observation(ChangeObservation {
                id: "fingerprint-change".into(),
                repository_id: "repo".into(),
                worktree_id: work.workspace_id,
                work_id: Some(work.id),
                commit_id: "new-commit".into(),
                parent_commit_ids: Vec::new(),
                branch: None,
                timestamp: Utc::now(),
                files: Vec::new(),
            })
            .unwrap();
        let after = crate::resolve_conversation_context_with_graph(
            &store,
            conversation_id,
            crate::ResolveConversationOptions::default(),
        )
        .unwrap();
        assert_ne!(before.context_fingerprint, after.context_fingerprint);
        assert!(after.graph_context.iter().any(|reference| {
            reference.kind == ContextNodeKind::Commit && reference.canonical_id == "new-commit"
        }));
    }
}
