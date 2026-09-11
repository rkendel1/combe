use crate::chrome_view::ClickView;
use combe_state::{ParticipantKind, TurnOrigin, WorkContext};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSColor, NSFont, NSLineBreakMode, NSScrollView,
    NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

pub struct Actions {
    pub new_proposal: Box<dyn Fn()>,
    pub review: Box<dyn Fn()>,
    pub approve: Box<dyn Fn()>,
    pub reject: Box<dyn Fn()>,
    pub assign: Box<dyn Fn()>,
    pub export: Box<dyn Fn()>,
    pub import: Box<dyn Fn()>,
    pub handoff: Option<Box<dyn Fn()>>,
}

pub fn new(
    mtm: MainThreadMarker,
    frame: NSRect,
    context: &WorkContext,
    actions: Actions,
) -> Retained<NSScrollView> {
    let width = frame.size.width.clamp(340.0, 380.0);
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
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(&text), mtm);
    field.setFrame(NSRect::new(
        NSPoint::new(24.0, 24.0),
        NSSize::new(width - 48.0, frame.size.height.max(640.0) - 124.0),
    ));
    field.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    field.setTextColor(Some(&NSColor::labelColor()));
    field.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);

    let document = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width, frame.size.height.max(640.0)),
        ),
    );
    document.addSubview(&field);
    let action_y = frame.size.height.max(640.0) - 48.0;
    let new_proposal = ClickView::new(
        mtm,
        NSRect::new(NSPoint::new(16.0, action_y), NSSize::new(96.0, 32.0)),
        "New Proposal",
        12.0,
        12.0,
        actions.new_proposal,
    );
    document.addSubview(&new_proposal);
    for (x, width, label, action) in [
        (116.0, 64.0, "Review", actions.review),
        (184.0, 72.0, "Approve", actions.approve),
        (260.0, 64.0, "Reject", actions.reject),
    ] {
        let button = ClickView::new(
            mtm,
            NSRect::new(NSPoint::new(x, action_y), NSSize::new(width, 32.0)),
            label,
            12.0,
            12.0,
            action,
        );
        document.addSubview(&button);
    }
    let secondary_y = action_y - 36.0;
    let assign = ClickView::new(
        mtm,
        NSRect::new(NSPoint::new(16.0, secondary_y), NSSize::new(58.0, 32.0)),
        "Assign",
        12.0,
        12.0,
        actions.assign,
    );
    document.addSubview(&assign);
    let export = ClickView::new(
        mtm,
        NSRect::new(NSPoint::new(78.0, secondary_y), NSSize::new(88.0, 32.0)),
        "Export Context",
        12.0,
        12.0,
        actions.export,
    );
    export.setAccessibilityLabel(Some(&NSString::from_str("Export Work Context")));
    document.addSubview(&export);
    let import = ClickView::new(
        mtm,
        NSRect::new(NSPoint::new(170.0, secondary_y), NSSize::new(72.0, 32.0)),
        "Import",
        12.0,
        12.0,
        actions.import,
    );
    import.setAccessibilityLabel(Some(&NSString::from_str("Import Work Contribution")));
    document.addSubview(&import);
    if let Some(handoff) = actions.handoff {
        let button = ClickView::new(
            mtm,
            NSRect::new(NSPoint::new(246.0, secondary_y), NSSize::new(78.0, 32.0)),
            "Handoff…",
            12.0,
            12.0,
            handoff,
        );
        button.setAccessibilityLabel(Some(&NSString::from_str("Handoff Work")));
        document.addSubview(&button);
    }
    panel.setDocumentView(Some(&document));
    panel
}

fn content(context: &WorkContext) -> String {
    let mut text = format!("{}\n{:?}\n", context.work.title, context.work.status);
    if let Some(objective) = &context.work.objective {
        text.push_str(&format!("\n{objective}\n"));
    }
    text.push_str("\nPROPOSALS\n");
    for proposal in &context.state.active_proposals {
        text.push_str(&format!(
            "{}\n{:?} · {}\n",
            proposal.title, proposal.status, proposal.proposed_by
        ));
    }
    text.push_str("\nAPPROVED\n");
    for proposal in &context.state.approved_proposals {
        text.push_str(&format!("{} · Decision recorded\n", proposal.title));
    }
    text.push_str("\nEXECUTING\n");
    for assignment in &context.state.active_assignments {
        text.push_str(&format!("{}\n", assignment.instruction));
    }
    text.push_str("\nRECENT RESULTS\n");
    for turn in &context.state.recent_results {
        text.push_str(&format!("{}\n", turn.content));
    }
    text.push_str("\nDECISIONS\n");
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
    text
}
