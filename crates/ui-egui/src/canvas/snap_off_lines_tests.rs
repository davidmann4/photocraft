//! #16857: with View > Snap off and Smart Guides on, a Move-tool drag that lines an edge or centre up exactly with
//! another layer's still draws that smart guide, with no pull; Smart Guides off draws nothing, and Snap on keeps
//! snapping.

use craft::behavior::Axis;
use craft::behavior::geometry::SnapLine;
use serde_json::json;

use crate::canvas::{ToolEvent, tool_event, tool_event_response};
use crate::state::Tool;
use crate::{PhotocraftApp, Services};

/// The epic's test document: 1000 x 800, white Background, A red (100,100)-(300,260), C green (400,100)-(500,200)
/// and B blue (600,400)-(750,520) on top and active; the Move tool without Auto-Select, the view fitted (the test
/// window is smaller than the document at 100 %; an exact coincidence does not depend on the zoom).
fn test_document(snap: bool, smart_guides: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 1000, "height": 800})).unwrap();
    app.sync_views();
    for (name, rect, color) in [("A", [100, 100, 200, 160], "#ff0000"), ("C", [400, 100, 100, 100], "#00ff00"), ("B", [600, 400, 150, 120], "#0000ff")] {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = snap;
    app.ui.view.show.smart_guides = smart_guides;
    app
}

/// B's bounds.
fn b_left(app: &PhotocraftApp) -> i32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds().x0
}

/// Drags B from its centre (675, 460) by `dx` and returns the snap lines of the last move; B's left edge after the
/// release.
fn drag_b(app: &mut PhotocraftApp, dx: f64) -> (Vec<SnapLine>, i32) {
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 675.0, y: 460.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: 675.0 + dx / 2.0, y: 460.0, pressure: 1.0 }, m);
    let lines = tool_event_response(app, ToolEvent::Move { x: 675.0 + dx, y: 460.0, pressure: 1.0 }, m).snapped;
    tool_event(app, ToolEvent::Up { x: 675.0 + dx, y: 460.0 }, m);
    (lines, b_left(app))
}

fn vertical_at(lines: &[SnapLine], x: f64) -> bool {
    lines.iter().any(|l| l.along == Axis::Y && l.position == x && l.kind.is_smart())
}

#[test]
fn snap_off_draws_a_smart_guide_where_an_edge_lines_up_exactly() {
    let mut app = test_document(false, true);
    let (lines, left) = drag_b(&mut app, -500.0);
    assert_eq!(left, 100, "B's left edge on A's");
    assert!(vertical_at(&lines, 100.0), "x=100 (A's left) is drawn: {lines:?}");
}

#[test]
fn snap_off_draws_nothing_and_does_not_pull_four_pixels_away() {
    let mut app = test_document(false, true);
    let (lines, left) = drag_b(&mut app, -496.0);
    assert_eq!(left, 104, "no pull with Snap off");
    assert!(!vertical_at(&lines, 100.0), "nothing for A's left 4 px away: {lines:?}");
}

#[test]
fn snap_off_and_smart_guides_off_draw_nothing() {
    let mut app = test_document(false, false);
    let (lines, left) = drag_b(&mut app, -500.0);
    assert_eq!(left, 100);
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn snap_off_ctrl_draws_nothing() {
    let mut app = test_document(false, true);
    let m = egui::Modifiers::NONE;
    let ctrl = if cfg!(target_os = "macos") { egui::Modifiers::CTRL } else { egui::Modifiers::COMMAND };
    tool_event(&mut app, ToolEvent::Down { x: 675.0, y: 460.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 425.0, y: 460.0, pressure: 1.0 }, ctrl);
    let lines = tool_event_response(&mut app, ToolEvent::Move { x: 175.0, y: 460.0, pressure: 1.0 }, ctrl).snapped;
    tool_event(&mut app, ToolEvent::Up { x: 175.0, y: 460.0 }, ctrl);
    assert_eq!(b_left(&app), 100);
    assert!(lines.is_empty(), "the suspend key hides the lines too: {lines:?}");
}

#[test]
fn snap_on_keeps_snapping_and_its_line() {
    let mut app = test_document(true, true);
    let (lines, left) = drag_b(&mut app, -496.0);
    assert_eq!(left, 100, "Snap on pulls B's left onto A's");
    assert!(vertical_at(&lines, 100.0), "{lines:?}");
}
