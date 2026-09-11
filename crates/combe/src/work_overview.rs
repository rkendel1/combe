use crate::chrome_view::ClickView;
use combe_state::{ParticipantKind, WorkContext};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSColor, NSFont, NSLineBreakMode, NSScrollView,
    NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

pub fn new(
    mtm: MainThreadMarker,
    frame: NSRect,
    context: &WorkContext,
    handoff: Option<Box<dyn Fn()>>,
) -> Retained<NSScrollView> {
    let width = frame.size.width.min(380.0).max(280.0);
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
        NSSize::new(width - 48.0, frame.size.height.max(640.0) - 48.0),
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
    if let Some(handoff) = handoff {
        let button = ClickView::new(
            mtm,
            NSRect::new(
                NSPoint::new(20.0, frame.size.height.max(640.0) - 48.0),
                NSSize::new(100.0, 32.0),
            ),
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
    text.push_str("\nPARTICIPANTS\n");
    for participant in &context.participants {
        let kind = match participant.kind {
            ParticipantKind::Human => "Human",
            ParticipantKind::Agent => "Agent",
            ParticipantKind::System => "System",
        };
        text.push_str(&format!("{} · {kind}\n", participant.name));
    }
    text.push_str("\nACTIVE ASSIGNMENTS\n");
    for assignment in &context.active_assignments {
        text.push_str(&format!("{}\n", assignment.instruction));
    }
    text.push_str("\nDECISIONS\n");
    for decision in &context.decisions {
        text.push_str(&format!("{}\n", decision.statement));
    }
    text.push_str("\nRECENT TURNS\n");
    for turn in &context.recent_turns {
        text.push_str(&format!("{:?}: {}\n", turn.kind, turn.content));
    }
    text
}
