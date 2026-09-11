use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AiContinuityStore, AiExchangeStatus, AiProviderExchange, ContextGraphProjector,
    ContextGraphStore, ExecutionStatus, GraphHealth, ProposalStatus, Result, WorkContext, WorkId,
    WorkStatus, WorkStore,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkAttention {
    pub work_id: WorkId,
    pub revision: u64,
    pub items: Vec<AttentionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionItem {
    pub kind: AttentionKind,
    pub priority: AttentionPriority,
    pub title: String,
    pub reason: String,
    pub related_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionPriority {
    Critical,
    High,
    Normal,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    ResolveFailure,
    RebuildGraph,
    ReviewResult,
    ReviewProposal,
    ApproveProposal,
    AssignWork,
    ReviewExecution,
    ReviewArtifact,
    RefreshContext,
    CompleteWork,
}

pub fn derive_work_attention<S>(store: &S, work_id: &WorkId) -> Result<WorkAttention>
where
    S: WorkStore + AiContinuityStore + ContextGraphStore,
{
    let health = ContextGraphProjector::verify(store)?;
    derive_work_attention_with_health(store, work_id, &health)
}

pub fn derive_work_attention_with_health<S>(
    store: &S,
    work_id: &WorkId,
    health: &GraphHealth,
) -> Result<WorkAttention>
where
    S: WorkStore + AiContinuityStore,
{
    let context = store.context(work_id)?;
    let exchanges = store.ai_exchanges(&format!("work:{}", work_id.0))?;
    Ok(project_attention(
        context,
        exchanges,
        health,
        store.current_revision()?,
    ))
}

fn project_attention(
    context: WorkContext,
    exchanges: Vec<AiProviderExchange>,
    health: &GraphHealth,
    revision: u64,
) -> WorkAttention {
    let mut items = Vec::new();
    if context.work.status != WorkStatus::Completed && context.work.status != WorkStatus::Archived {
        let mut latest = BTreeMap::new();
        for exchange in exchanges {
            let key = exchange
                .recipient_id
                .as_ref()
                .map(|id| id.0.clone())
                .unwrap_or_else(|| exchange.provider.clone());
            latest.insert(key, exchange);
        }
        for exchange in latest.into_values() {
            match exchange.status {
                AiExchangeStatus::Failed => items.push(AttentionItem {
                    kind: AttentionKind::ResolveFailure,
                    priority: AttentionPriority::Critical,
                    title: "Resolve provider failure".into(),
                    reason: exchange
                        .error
                        .clone()
                        .unwrap_or_else(|| format!("{} failed", exchange.provider)),
                    related_id: Some(exchange.id),
                }),
                AiExchangeStatus::Completed
                    if !context
                        .proposals
                        .iter()
                        .any(|proposal| proposal.created_at > exchange.created_at)
                        && !context
                            .reviews
                            .iter()
                            .any(|review| review.created_at > exchange.created_at)
                        && !context
                            .execution_reviews
                            .iter()
                            .any(|review| review.created_at > exchange.created_at) =>
                {
                    items.push(AttentionItem {
                        kind: AttentionKind::ReviewResult,
                        priority: AttentionPriority::High,
                        title: "Review provider result".into(),
                        reason: format!(
                            "{} completed with {} context items · {}",
                            exchange.provider,
                            exchange.context_items.len(),
                            short_fingerprint(&exchange.context_fingerprint)
                        ),
                        related_id: Some(exchange.id),
                    });
                }
                _ => {}
            }
        }
        for proposal in context
            .proposals
            .iter()
            .filter(|proposal| proposal.status == ProposalStatus::Proposed)
        {
            items.push(AttentionItem {
                kind: AttentionKind::ReviewProposal,
                priority: AttentionPriority::High,
                title: "Review proposal".into(),
                reason: proposal.title.clone(),
                related_id: Some(proposal.id.0.clone()),
            });
        }
        for proposal in &context.state.approved_proposals {
            if !context
                .assignments
                .iter()
                .any(|assignment| assignment.proposal_id.as_ref() == Some(&proposal.id))
            {
                items.push(AttentionItem {
                    kind: AttentionKind::AssignWork,
                    priority: AttentionPriority::Normal,
                    title: "Assign approved work".into(),
                    reason: proposal.title.clone(),
                    related_id: Some(proposal.id.0.clone()),
                });
            }
        }
        for execution in &context.executions {
            if execution.status == ExecutionStatus::Failed
                || execution.status == ExecutionStatus::Cancelled
                || context
                    .state
                    .stale_executions
                    .iter()
                    .any(|stale| stale.id == execution.id)
            {
                items.push(AttentionItem {
                    kind: AttentionKind::ResolveFailure,
                    priority: AttentionPriority::Critical,
                    title: "Resolve execution failure".into(),
                    reason: execution
                        .failure
                        .clone()
                        .unwrap_or_else(|| format!("Execution is {:?}", execution.status)),
                    related_id: Some(execution.id.0.clone()),
                });
            } else if execution.status == ExecutionStatus::Completed
                && !context
                    .execution_reviews
                    .iter()
                    .any(|review| review.execution_id == execution.id)
            {
                items.push(AttentionItem {
                    kind: AttentionKind::ReviewExecution,
                    priority: AttentionPriority::High,
                    title: "Review completed execution".into(),
                    reason: format!("{} completed", execution.provider),
                    related_id: Some(execution.id.0.clone()),
                });
            }
        }
        if !health.healthy {
            items.push(AttentionItem {
                kind: AttentionKind::RebuildGraph,
                priority: AttentionPriority::High,
                title: "Rebuild context graph".into(),
                reason: if health.stale {
                    "Context Graph is stale".into()
                } else {
                    "Context Graph failed verification".into()
                },
                related_id: health.projection_fingerprint.clone(),
            });
        }
        if items.is_empty()
            && !context.executions.is_empty()
            && context
                .executions
                .iter()
                .all(|execution| execution.status == ExecutionStatus::Completed)
        {
            items.push(AttentionItem {
                kind: AttentionKind::CompleteWork,
                priority: AttentionPriority::Low,
                title: "Complete Work".into(),
                reason: "All executions have completed and been reviewed".into(),
                related_id: Some(context.work.id.0.clone()),
            });
        }
    }
    items.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(left.kind.cmp(&right.kind))
            .then(left.related_id.cmp(&right.related_id))
    });
    WorkAttention {
        work_id: context.work.id,
        revision,
        items,
    }
}

fn short_fingerprint(value: &str) -> String {
    value.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        FeltDbWorkStore, Participant, ParticipantKind, ProposalId, TurnOrigin, Work, WorkProposal,
    };

    #[test]
    fn proposed_work_has_deterministic_review_attention() {
        let directory = TempDir::new().unwrap();
        let store = FeltDbWorkStore::open(directory.path().join("attention.db")).unwrap();
        let work = Work::new("/tmp/worktree".into(), "Auth routing".into(), None);
        let human = Participant::new(work.id.clone(), ParticipantKind::Human, "Human".into());
        store
            .create_work(work.clone(), vec![human.clone()])
            .unwrap();
        let now = Utc::now();
        store
            .create_proposal(WorkProposal {
                id: ProposalId::new(),
                work_id: work.id.clone(),
                proposed_by: human.id,
                title: "Add recipient registry".into(),
                statement: "Add the approved registry boundary".into(),
                rationale: None,
                status: ProposalStatus::Proposed,
                origin: TurnOrigin::Local,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        ContextGraphProjector::rebuild(&store).unwrap();

        let first = derive_work_attention(&store, &work.id).unwrap();
        let second = derive_work_attention(&store, &work.id).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].kind, AttentionKind::ReviewProposal);
        assert_eq!(first.items[0].priority, AttentionPriority::High);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
    }
}
