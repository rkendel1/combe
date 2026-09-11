use crate::chrome_view::ClickView;
use chrono::{DateTime, Utc};
use combe_state::{
    ExecutionStatus, ParticipantKind, ProposalStatus, RecipientId, TurnOrigin, WorkContext,
    WorkStatus,
};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSColor, NSFont, NSLineBreakMode, NSPopUpButton, NSScrollView,
    NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

pub struct Actions {
    pub send: Box<dyn Fn(RecipientId, String)>,
    pub configure_recipients: Box<dyn Fn()>,
    pub new_proposal: Box<dyn Fn()>,
    pub review: Box<dyn Fn()>,
    pub request_changes: Box<dyn Fn()>,
    pub approve: Box<dyn Fn()>,
    pub reject: Box<dyn Fn()>,
    pub assign: Box<dyn Fn()>,
    pub export: Box<dyn Fn()>,
    pub import: Box<dyn Fn()>,
    pub chatgpt_prepare: Box<dyn Fn()>,
    pub chatgpt_propose: Box<dyn Fn()>,
    pub chatgpt_review: Box<dyn Fn()>,
    pub chatgpt_import: Box<dyn Fn()>,
    pub open_terminal: Box<dyn Fn()>,
    pub handoff: Option<Box<dyn Fn()>>,
}

type ActionButton = (&'static str, Box<dyn Fn()>, bool);

#[derive(Clone)]
pub struct RouteOption {
    pub recipient_id: RecipientId,
    pub label: String,
}

pub fn new(
    mtm: MainThreadMarker,
    frame: NSRect,
    context: &WorkContext,
    routes: &[RouteOption],
    actions: Actions,
) -> Retained<NSScrollView> {
    let width = frame.size.width.clamp(460.0, 560.0);
    let panel_frame = NSRect::new(
        NSPoint::new(frame.size.width - width, 0.0),
        NSSize::new(width, frame.size.height),
    );
    let panel = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), panel_frame);
    panel.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    panel.setDrawsBackground(true);
    panel.setBackgroundColor(&NSColor::windowBackgroundColor());
    panel.setHasVerticalScroller(true);

    let text = content(context);
    let document_height = ((text.lines().count() as f64 * 18.0) + 500.0)
        .max(frame.size.height)
        .max(640.0);
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(&text), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(24.0, 24.0),
        NSSize::new(width - 48.0, document_height - 460.0),
    ));
    field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    field.setTextColor(Some(&NSColor::labelColor()));
    field.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);

    let document = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, document_height)),
    );
    document.addSubview(&field);
    let Actions {
        send,
        configure_recipients,
        new_proposal,
        review,
        request_changes,
        approve,
        reject,
        assign,
        export,
        import,
        chatgpt_prepare,
        chatgpt_propose,
        chatgpt_review,
        chatgpt_import,
        open_terminal,
        handoff,
    } = actions;
    let composer_top = document_height - 44.0;
    let to_label = NSTextField::labelWithString(&NSString::from_str("TO"), mtm);
    to_label.setFrame(NSRect::new(
        NSPoint::new(20.0, composer_top),
        NSSize::new(width - 40.0, 18.0),
    ));
    to_label.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
    document.addSubview(&to_label);
    let picker = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        NSRect::new(
            NSPoint::new(16.0, composer_top - 34.0),
            NSSize::new(width - 32.0, 28.0),
        ),
        false,
    );
    for route in routes {
        picker.addItemWithTitle(&NSString::from_str(&route.label));
    }
    picker.setEnabled(!routes.is_empty());
    document.addSubview(&picker);
    let route_detail = if routes.is_empty() {
        "No recipients configured"
    } else {
        "The selected recipient determines the provider, model, and delivery method."
    };
    let detail = NSTextField::labelWithString(&NSString::from_str(route_detail), mtm);
    detail.setFrame(NSRect::new(
        NSPoint::new(20.0, composer_top - 58.0),
        NSSize::new(width - 40.0, 18.0),
    ));
    detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
    detail.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    document.addSubview(&detail);
    let message_label = NSTextField::labelWithString(&NSString::from_str("MESSAGE"), mtm);
    message_label.setFrame(NSRect::new(
        NSPoint::new(20.0, composer_top - 88.0),
        NSSize::new(width - 40.0, 18.0),
    ));
    message_label.setFont(Some(&NSFont::boldSystemFontOfSize(11.0)));
    document.addSubview(&message_label);
    let message = NSTextField::textFieldWithString(&NSString::from_str(""), mtm);
    message.setFrame(NSRect::new(
        NSPoint::new(16.0, composer_top - 128.0),
        NSSize::new(width - 32.0, 34.0),
    ));
    message.setPlaceholderString(Some(&NSString::from_str(
        "Describe what this recipient should do…",
    )));
    document.addSubview(&message);
    let route_ids = routes
        .iter()
        .map(|route| route.recipient_id.clone())
        .collect::<Vec<_>>();
    let picker_for_send = picker.clone();
    let message_for_send = message.clone();
    let send_button = if routes.is_empty() {
        ClickView::new(
            mtm,
            NSRect::new(
                NSPoint::new(16.0, composer_top - 174.0),
                NSSize::new(width - 32.0, 36.0),
            ),
            "Configure a Recipient…",
            12.0,
            12.0,
            configure_recipients,
        )
    } else {
        ClickView::new(
            mtm,
            NSRect::new(
                NSPoint::new(16.0, composer_top - 174.0),
                NSSize::new(width - 32.0, 36.0),
            ),
            "Continue with Selected Recipient…",
            12.0,
            12.0,
            move || {
                let index = usize::try_from(picker_for_send.indexOfSelectedItem()).ok();
                if let Some(recipient_id) = index.and_then(|index| route_ids.get(index)).cloned() {
                    send(recipient_id, message_for_send.stringValue().to_string());
                }
            },
        )
    };
    send_button.set_corner_radius(8.0);
    send_button.set_selected(true);
    document.addSubview(&send_button);
    let proposed = context
        .proposals
        .iter()
        .any(|proposal| proposal.status == ProposalStatus::Proposed);
    let approved_unassigned = context.state.approved_proposals.iter().any(|proposal| {
        !context
            .assignments
            .iter()
            .any(|assignment| assignment.proposal_id.as_ref() == Some(&proposal.id))
    });
    let reviewable = context.executions.iter().any(|execution| {
        execution.status != ExecutionStatus::Started
            && !context
                .execution_reviews
                .iter()
                .any(|review| review.execution_id == execution.id)
    });
    let mut buttons: Vec<ActionButton> = Vec::new();
    if context.proposals.is_empty() {
        buttons.push(("Create First Proposal…", new_proposal, true));
        buttons.push(("Prepare ChatGPT Proposal", chatgpt_propose, false));
        buttons.push(("Import ChatGPT Response…", chatgpt_import, false));
        buttons.push(("Open Workspace Terminal", open_terminal, false));
    } else if proposed {
        buttons.push(("Approve Proposal", approve, true));
        buttons.push(("Request Changes…", request_changes, false));
        buttons.push(("Reject Proposal", reject, false));
        buttons.push(("Prepare ChatGPT Review", chatgpt_review, false));
        buttons.push(("Import Contribution…", import, false));
    } else if approved_unassigned {
        buttons.push(("Assign Approved Work…", assign, true));
        buttons.push(("Create Another Proposal…", new_proposal, false));
        buttons.push(("Open Workspace Terminal", open_terminal, false));
    } else if reviewable {
        buttons.push(("Review Completed Work…", review, true));
        buttons.push(("Prepare ChatGPT Review", chatgpt_review, false));
        buttons.push(("Import ChatGPT Response…", chatgpt_import, false));
        buttons.push(("Open Workspace Terminal", open_terminal, false));
    } else if !context.state.active_executions.is_empty() || handoff.is_some() {
        if let Some(handoff) = handoff {
            buttons.push(("Run Assigned Work…", handoff, true));
            buttons.push(("Open Workspace Terminal", open_terminal, false));
        } else {
            buttons.push(("Open Workspace Terminal", open_terminal, true));
        }
        buttons.push(("Prepare ChatGPT Context", chatgpt_prepare, false));
    } else {
        buttons.push(("Create Another Proposal…", new_proposal, true));
        buttons.push(("Open Workspace Terminal", open_terminal, false));
    }
    buttons.push(("Export Context", export, false));
    let button_width = (width - 44.0) / 2.0;
    for (index, (label, action, primary)) in buttons.into_iter().enumerate() {
        let column = index % 2;
        let row = index / 2;
        let button = ClickView::new(
            mtm,
            NSRect::new(
                NSPoint::new(
                    16.0 + column as f64 * (button_width + 12.0),
                    document_height - 274.0 - row as f64 * 42.0,
                ),
                NSSize::new(button_width, 34.0),
            ),
            label,
            12.0,
            12.0,
            action,
        );
        button.set_corner_radius(8.0);
        button.set_selected(primary);
        document.addSubview(&button);
    }
    panel.setDocumentView(Some(&document));
    panel
}

pub(crate) fn content(context: &WorkContext) -> String {
    let status = match context.work.status {
        WorkStatus::Active => "● Active",
        WorkStatus::Paused => "◐ Paused",
        WorkStatus::Completed => "✓ Completed",
        WorkStatus::Archived => "— Archived",
    };
    let mut text = format!("{}\n{status}\n", context.work.title);
    if let Some(objective) = &context.work.objective {
        text.push_str(&format!("\n{objective}\n"));
    }
    text.push_str(&format!(
        "Updated {}\n",
        context.work.updated_at.format("%Y-%m-%d %H:%M")
    ));
    if context.proposals.is_empty()
        && context.assignments.is_empty()
        && context.executions.is_empty()
        && context.decisions.is_empty()
        && context.artifacts.is_empty()
        && context.conversations.is_empty()
        && context.recent_turns.is_empty()
    {
        text.push_str(
            "\nNEXT\nCreate the first proposal. Nothing runs until you approve and assign it.\n\nHOW WORK MOVES\n1. Create a proposal describing the change.\n2. Approve it or request changes.\n3. Assign approved work to an execution participant.\n4. Run it in the workspace terminal.\n5. Review the durable result and artifacts.\n\nCHATGPT (OPTIONAL)\nTransfer is manual: prepare context here, paste it into ChatGPT, then import the copied response.\n\nWORKSPACE\n",
        );
        text.push_str(&context.work.workspace_id);
        text.push_str("\n\nPARTICIPANTS\n");
        for participant in &context.participants {
            text.push_str(&format!("{}\n", participant.name));
        }
        return text;
    }
    text.push_str("\nNEXT\n");
    text.push_str(next_action(context));
    text.push_str("\n\nCOORDINATION\n");
    let pending_reviews = context
        .proposals
        .iter()
        .filter(|proposal| proposal.status == ProposalStatus::Proposed)
        .count();
    let active_executions = context
        .executions
        .iter()
        .filter(|execution| execution.status == ExecutionStatus::Started)
        .count();
    text.push_str(&format!(
        "{} proposals · {} pending review\n{} assignments · {} active execution\n{} decisions · {} artifacts\n",
        context.proposals.len(),
        pending_reviews,
        context.assignments.len(),
        active_executions,
        context.decisions.len(),
        context.artifacts.len()
    ));
    if let Some(execution) = context.executions.last() {
        let assignment = context
            .assignments
            .iter()
            .find(|assignment| assignment.id == execution.assignment_id);
        let proposal = assignment
            .and_then(|assignment| assignment.proposal_id.as_ref())
            .and_then(|proposal_id| {
                context
                    .proposals
                    .iter()
                    .find(|proposal| proposal.id == *proposal_id)
            });
        let decision = proposal.and_then(|proposal| {
            context
                .decisions
                .iter()
                .find(|decision| decision.proposal_id.as_ref() == Some(&proposal.id))
        });
        let review = context
            .execution_reviews
            .iter()
            .rev()
            .find(|review| review.execution_id == execution.id);
        text.push_str(&format!(
            "Proposal\n{}\n{}\nExecution\n{} · {:?}\nReview\n{}\nNext\n{}\n",
            proposal
                .map(|proposal| proposal.title.as_str())
                .unwrap_or("Unlinked"),
            decision
                .map(|decision| format!("Approved by {}", decision.decided_by))
                .unwrap_or_else(|| "No approval linked".into()),
            execution.provider,
            execution.status,
            review
                .map(|review| review.content.as_str())
                .unwrap_or("Awaiting review"),
            if review.is_some() {
                "Inspect review"
            } else {
                "Review execution"
            }
        ));
    } else {
        text.push_str("No execution yet\nNext\nAssign approved work\n");
    }
    text.push_str("\nCHATGPT\nConversation\n");
    let chatgpt_conversation = context.conversations.iter().find(|conversation| {
        conversation.conversation.provider == combe_state::ConversationProvider::ChatGpt
    });
    text.push_str(
        chatgpt_conversation
            .and_then(|conversation| {
                conversation
                    .label
                    .as_deref()
                    .or(conversation.conversation.title.as_deref())
            })
            .unwrap_or("Not linked"),
    );
    text.push_str("\nManual transfer · no direct transport\nActions\nPrepare Context · Propose · Review · Import Response\n");
    text.push_str("\nPROPOSALS\n");
    if context.proposals.is_empty() {
        text.push_str("No proposals yet.\n");
    }
    for proposal in &context.proposals {
        let author = participant_name(context, &proposal.proposed_by.0);
        text.push_str(&format!(
            "{}\n{:?} · Proposed by {}\n",
            proposal.title, proposal.status, author
        ));
    }
    text.push_str("\nAPPROVED\n");
    for proposal in &context.state.approved_proposals {
        text.push_str(&format!("{} · Decision recorded\n", proposal.title));
    }
    text.push_str("\nASSIGNED\n");
    if context.assignments.is_empty() {
        text.push_str("No assignments yet.\n");
    }
    for assignment in &context.state.assigned {
        text.push_str(&format!("{}\n", assignment.instruction));
    }
    for (heading, executions) in [
        ("ACTIVE EXECUTIONS", &context.state.active_executions),
        ("STALE EXECUTIONS", &context.state.stale_executions),
        ("COMPLETED WORK", &context.state.completed_executions),
        ("FAILED WORK", &context.state.failed_executions),
        ("CANCELLED WORK", &context.state.cancelled_executions),
    ] {
        text.push_str(&format!("\n{heading}\n"));
        if executions.is_empty() {
            text.push_str("None\n");
        }
        for execution in executions {
            let assignment = context
                .assignments
                .iter()
                .find(|assignment| assignment.id == execution.assignment_id);
            let result = execution
                .result_id
                .as_ref()
                .and_then(|id| context.results.iter().find(|result| result.id == *id));
            text.push_str(&format!(
                "{:?} · {}\nAssignment {} · {}\nProvider {}{}\nResult {}{}\n",
                execution.status,
                assignment
                    .map(|assignment| assignment.instruction.as_str())
                    .unwrap_or("Execution"),
                execution.assignment_id,
                execution.participant_id,
                execution.provider,
                execution
                    .provider_execution_id
                    .as_ref()
                    .map(|id| format!(" · {id}"))
                    .unwrap_or_default(),
                result
                    .and_then(|result| result.summary.as_deref())
                    .or(execution.result_id.as_deref())
                    .unwrap_or("none"),
                execution
                    .failure
                    .as_ref()
                    .map(|failure| format!(" · {failure}"))
                    .unwrap_or_default()
            ));
        }
    }
    text.push_str("\nDECISIONS\n");
    if context.decisions.is_empty() {
        text.push_str("No decisions recorded.\n");
    }
    for decision in &context.decisions {
        text.push_str(&format!("{}\n", decision.statement));
    }
    text.push_str("\nCONVERSATIONS\n");
    for conversation in &context.conversations {
        let turn_count = context
            .recent_turns
            .iter()
            .filter(|turn| {
                matches!(
                    &turn.origin,
                    TurnOrigin::ExternalConversation(reference)
                        if reference.id == conversation.conversation.id
                            && reference.provider == conversation.conversation.provider
                )
            })
            .count();
        let label = conversation
            .label
            .as_deref()
            .or(conversation.conversation.title.as_deref())
            .unwrap_or(&conversation.conversation.id);
        text.push_str(&format!(
            "{:?} · {label} · {turn_count} imported turns\n",
            conversation.conversation.provider
        ));
    }
    text.push_str("\nPARTICIPANTS\n");
    if context.participants.is_empty() {
        text.push_str("No participants recorded.\n");
    }
    for participant in &context.participants {
        let kind = match participant.kind {
            ParticipantKind::Human => "Human",
            ParticipantKind::Agent => "Agent",
            ParticipantKind::System => "System",
        };
        text.push_str(&format!("{} · {kind}\n", participant.name));
    }
    text.push_str("\nRECENT TURNS\n");
    for turn in &context.recent_turns {
        text.push_str(&format!("{:?}: {}\n", turn.kind, turn.content));
    }
    text.push_str("\nEXECUTION ENVIRONMENT\nWorkspace / Worktree\n");
    text.push_str(&context.work.workspace_id);
    text.push_str("\nTerminal\nAvailable in this Combe workspace\n");
    text.push_str("\nARTIFACTS\n");
    if context.artifacts.is_empty() {
        text.push_str("No artifacts recorded.\n");
    }
    for artifact in &context.artifacts {
        text.push_str(&format!(
            "{:?} · {}{}\n",
            artifact.kind,
            artifact.path.as_deref().unwrap_or("No path"),
            artifact
                .description
                .as_ref()
                .map(|description| format!(" · {description}"))
                .unwrap_or_default()
        ));
    }
    text.push_str("\nACTIVITY\n");
    let activity = activity(context);
    if activity.is_empty() {
        text.push_str("No activity yet.\n");
    }
    for item in activity {
        text.push_str(&format!(
            "{}  {}\n",
            item.at.format("%H:%M"),
            item.description
        ));
    }
    text
}

fn participant_name<'a>(context: &'a WorkContext, id: &'a str) -> &'a str {
    context
        .participants
        .iter()
        .find(|participant| participant.id.0 == id)
        .map(|participant| participant.name.as_str())
        .unwrap_or(id)
}

pub(crate) fn next_action(context: &WorkContext) -> &'static str {
    if !context.state.failed_executions.is_empty() || !context.state.stale_executions.is_empty() {
        "Execution requires intervention."
    } else if context
        .proposals
        .iter()
        .any(|proposal| proposal.status == ProposalStatus::Proposed)
    {
        "Proposal awaiting your review."
    } else if context.executions.iter().any(|execution| {
        execution.status != ExecutionStatus::Started
            && !context
                .execution_reviews
                .iter()
                .any(|review| review.execution_id == execution.id)
    }) {
        "Execution completed and is awaiting review."
    } else if context.state.approved_proposals.iter().any(|proposal| {
        !context
            .assignments
            .iter()
            .any(|assignment| assignment.proposal_id.as_ref() == Some(&proposal.id))
    }) {
        "Approved proposal is awaiting assignment."
    } else if !context.state.active_executions.is_empty() {
        "An approved assignment is executing."
    } else if !context.state.completed_executions.is_empty() {
        "Completed work is available."
    } else if context.proposals.is_empty() {
        "Create the first proposal."
    } else {
        "Nothing requires attention."
    }
}

struct ActivityItem {
    at: DateTime<Utc>,
    description: String,
}

fn activity(context: &WorkContext) -> Vec<ActivityItem> {
    let mut items = Vec::new();
    for proposal in &context.proposals {
        items.push(ActivityItem {
            at: proposal.created_at,
            description: format!(
                "{} proposed {}",
                participant_name(context, &proposal.proposed_by.0),
                proposal.title
            ),
        });
    }
    for review in &context.reviews {
        items.push(ActivityItem {
            at: review.created_at,
            description: format!(
                "{} {:?} proposal {}",
                participant_name(context, &review.reviewed_by.0),
                review.outcome,
                review.proposal_id
            ),
        });
    }
    for assignment in &context.assignments {
        items.push(ActivityItem {
            at: assignment.created_at,
            description: format!(
                "Assignment sent to {}",
                participant_name(context, &assignment.to_participant_id.0)
            ),
        });
    }
    for decision in &context.decisions {
        items.push(ActivityItem {
            at: decision.created_at,
            description: format!(
                "{} decided {}",
                participant_name(context, &decision.decided_by.0),
                decision.statement
            ),
        });
    }
    for execution in &context.executions {
        items.push(ActivityItem {
            at: execution.started_at,
            description: format!("{} started execution", execution.provider),
        });
        if let Some(completed_at) = execution.completed_at {
            items.push(ActivityItem {
                at: completed_at,
                description: format!("{} {:?} execution", execution.provider, execution.status),
            });
        }
    }
    for artifact in &context.artifacts {
        items.push(ActivityItem {
            at: artifact.created_at,
            description: format!(
                "Artifact recorded: {}",
                artifact.path.as_deref().unwrap_or("reference")
            ),
        });
    }
    for review in &context.execution_reviews {
        items.push(ActivityItem {
            at: review.created_at,
            description: format!(
                "{} reviewed execution {}",
                participant_name(context, &review.reviewed_by.0),
                review.execution_id
            ),
        });
    }
    for turn in &context.recent_turns {
        items.push(ActivityItem {
            at: turn.created_at,
            description: format!(
                "{} contributed {:?}",
                participant_name(context, &turn.participant_id.0),
                turn.kind
            ),
        });
    }
    items.sort_by(|left, right| {
        left.at
            .cmp(&right.at)
            .then_with(|| left.description.cmp(&right.description))
    });
    let start = items.len().saturating_sub(20);
    items.drain(..start);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use combe_state::{
        ArtifactId, ArtifactKind, AssignmentId, AssignmentStatus, ExecutionId, Participant,
        ParticipantId, ParticipantKind, ProposalId, Work, WorkArtifact, WorkAssignment,
        WorkExecution, WorkProposal, WorkState,
    };

    fn context() -> WorkContext {
        let work = Work::new("/tmp/worktree".into(), "Native Work".into(), None);
        let human = Participant::new(work.id.clone(), ParticipantKind::Human, "Human".into());
        WorkContext {
            context_version: 2,
            revision: 1,
            work,
            participants: vec![human],
            recent_turns: Vec::new(),
            active_assignments: Vec::new(),
            decisions: Vec::new(),
            artifacts: Vec::new(),
            conversations: Vec::new(),
            proposals: Vec::new(),
            reviews: Vec::new(),
            state: WorkState::default(),
            executions: Vec::new(),
            assignments: Vec::new(),
            results: Vec::new(),
            execution_reviews: Vec::new(),
        }
    }

    #[test]
    fn empty_work_has_real_empty_states() {
        let text = content(&context());
        assert!(text.contains("Create the first proposal."));
        assert!(text.contains("Nothing runs until you approve and assign it."));
        assert!(text.contains("1. Create a proposal describing the change."));
        assert!(text.contains("Transfer is manual"));
    }

    #[test]
    fn next_action_follows_documented_precedence() {
        let mut context = context();
        let proposal = WorkProposal {
            id: ProposalId::new(),
            work_id: context.work.id.clone(),
            proposed_by: context.participants[0].id.clone(),
            title: "Approach".into(),
            statement: "Implement it".into(),
            rationale: None,
            status: ProposalStatus::Proposed,
            origin: TurnOrigin::Local,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        context.proposals.push(proposal.clone());
        context.state.active_proposals.push(proposal);
        assert_eq!(next_action(&context), "Proposal awaiting your review.");
        let assignment = WorkAssignment {
            id: AssignmentId::new(),
            work_id: context.work.id.clone(),
            from_participant_id: context.participants[0].id.clone(),
            to_participant_id: context.participants[0].id.clone(),
            instruction: "Run".into(),
            status: AssignmentStatus::Active,
            created_at: Utc::now(),
            proposal_id: None,
        };
        let execution = WorkExecution {
            id: ExecutionId::new(),
            work_id: context.work.id.clone(),
            assignment_id: assignment.id.clone(),
            participant_id: ParticipantId("agent".into()),
            provider: "codex".into(),
            provider_execution_id: None,
            status: ExecutionStatus::Failed,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            heartbeat_at: None,
            result_id: None,
            failure: Some("failed".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        context.assignments.push(assignment);
        context.executions.push(execution.clone());
        context.state.failed_executions.push(execution);
        assert_eq!(next_action(&context), "Execution requires intervention.");
    }

    #[test]
    fn projection_renders_participants_artifacts_and_bounded_activity() {
        let mut context = context();
        for index in 0..25 {
            context.artifacts.push(WorkArtifact {
                id: ArtifactId::new(),
                work_id: context.work.id.clone(),
                kind: ArtifactKind::File,
                path: Some(format!("file-{index}")),
                description: None,
                created_by: context.participants[0].id.clone(),
                created_at: Utc::now(),
            });
        }
        assert_eq!(activity(&context).len(), 20);
        let text = content(&context);
        assert!(text.contains("Human · Human"));
        assert!(text.contains("File · file-24"));
        assert!(text.contains("Workspace / Worktree\n/tmp/worktree"));
    }
}
