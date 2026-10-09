//! Crop tool gestures, Photoshop "Classic Mode" style (the frame moves over a fixed image):
//!
//! - outside the frame (or with no frame), a drag draws a new frame; ⇧ makes it square, ⌥ draws
//!   it from the centre, and holding Space while drawing repositions the frame being drawn;
//! - inside the frame, a drag moves it;
//! - on an edge or corner, a drag resizes it; ⇧ keeps the frame's aspect ratio, ⌥ resizes about
//!   the centre, and an options-bar ratio preset always holds.
//!
//! ↵ commits (`image.crop`, see `canvas::commit_crop`) and Esc cancels. The pending frame lives in
//! `UiState::crop_rect`, so the control channel reads it. `image.crop` has no angle, so dragging
//! outside the frame draws a new one rather than rotating it.
//!
//! As in Photoshop, from the first press until the crop is committed or cancelled the canvas also
//! shows the layers' pixels past its edges (kept by a crop with Delete Cropped Pixels off, or moved
//! out), with transparency out to the frame, under the shield ([`shows_beyond_canvas`]).

use egui::{CursorIcon, Modifiers};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// How far (screen points) from an edge the pointer still grabs it.
const HANDLE_PX: f64 = 8.0;
/// Smallest frame (document px) a gesture may leave behind.
const MIN_FRAME: f64 = 1.0;

/// Crop tool pointer state (not serialized: it only exists during a gesture).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CropState {
    pub drag: Option<CropDrag>,
    /// Space is held: drawing a new frame moves it instead of sizing it.
    pub space: bool,
    /// `UiState::crop_rect` is the untouched default frame ([`ensure_frame`]): a drag inside it
    /// draws a new frame (as in Photoshop) instead of moving it, and committing it does nothing.
    pub default_frame: bool,
    /// The document (index and size) the default frame was made for.
    frame_for: Option<(usize, u32, u32)>,
    /// The frame is being edited (pressed since it was made): the canvas shows what lies past it.
    pub editing: bool,
    /// The document the pending frame (or gesture) belongs to ([`cancel_stale`], #1918).
    doc: Option<photocraft_doc::DocId>,
}

/// A crop gesture in progress. Rects are `[x0, y0, x1, y1]` in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CropDrag {
    /// Drawing a new frame from `anchor`; `prev` is the frame it replaces, restored when the new
    /// one is too small (a click).
    Draw { anchor: [f64; 2], cur: [f64; 2], last: [f64; 2], prev: Option<[f64; 4]> },
    /// Moving the frame `rect` grabbed at `start`.
    Move { start: [f64; 2], rect: [f64; 4] },
    /// Dragging an edge or corner: `hx`/`hy` are -1 (left/top), 1 (right/bottom) or 0 (untouched).
    Resize { hx: i8, hy: i8, start: [f64; 2], rect: [f64; 4] },
}

/// What the pointer is over, relative to a crop frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Handle(i8, i8),
    Inside,
    Outside,
}

/// Hit-test a frame with `tol` document px of reach (shrunk for tiny frames so they stay movable).
pub fn hit(r: [f64; 4], p: [f64; 2], tol: f64) -> Hit {
    let axis = |lo: f64, hi: f64, v: f64, other_in: bool| -> i8 {
        let t = tol.min((hi - lo) / 4.0).max(0.0);
        // Outside the frame the full reach applies; inside it, the shrunk one.
        let near = |e: f64, outward: f64| (v - e) * outward <= tol && (e - v) * outward <= t;
        match (other_in, near(lo, -1.0), near(hi, 1.0)) {
            (false, ..) => 0,
            (_, true, true) => {
                if (v - lo).abs() <= (v - hi).abs() {
                    -1
                } else {
                    1
                }
            }
            (_, true, false) => -1,
            (_, false, true) => 1,
            _ => 0,
        }
    };
    let in_x = p[0] >= r[0] - tol && p[0] <= r[2] + tol;
    let in_y = p[1] >= r[1] - tol && p[1] <= r[3] + tol;
    let (hx, hy) = (axis(r[0], r[2], p[0], in_y), axis(r[1], r[3], p[1], in_x));
    if hx != 0 || hy != 0 {
        Hit::Handle(hx, hy)
    } else if p[0] > r[0] && p[0] < r[2] && p[1] > r[1] && p[1] < r[3] {
        Hit::Inside
    } else {
        Hit::Outside
    }
}

/// `[x0, y0, x1, y1]` with x0 ≤ x1, y0 ≤ y1.
fn normalized(a: [f64; 2], b: [f64; 2]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
}

fn size_ok(r: [f64; 4]) -> bool {
    r[2] - r[0] >= MIN_FRAME && r[3] - r[1] >= MIN_FRAME
}

/// The options-bar ratio preset as width / height.
fn preset_ratio(app: &PhotocraftApp) -> Option<f64> {
    let size = app.session.active().map_or((1.0, 1.0), |s| (s.doc.size.width as f64, s.doc.size.height as f64));
    crate::chrome_ui::crop_ratio(&app.ui.tool_options.crop_ratio, size.0, size.1).map(|(w, h)| w / h).filter(|k| k.is_finite() && *k > 0.0)
}

/// A new frame drawn from `anchor` to `cur`: ⇧ square, ⌥ from the centre, `ratio` held.
pub fn drawn(anchor: [f64; 2], cur: [f64; 2], ratio: Option<f64>, mods: Modifiers) -> [f64; 4] {
    let e = match ratio {
        Some(k) => crate::chrome_ui::marquee_end("fixedRatio", k, 1.0, false, anchor, cur),
        None => crate::chrome_ui::marquee_end("", 0.0, 0.0, mods.shift, anchor, cur),
    };
    if mods.alt { normalized([2.0 * anchor[0] - e[0], 2.0 * anchor[1] - e[1]], e) } else { normalized(anchor, e) }
}

/// Frame `r` with handle (`hx`, `hy`) dragged by `d`. `alt` resizes about the centre; `ratio`
/// (width / height) keeps the proportions.
pub fn resized(r: [f64; 4], hx: i8, hy: i8, d: [f64; 2], ratio: Option<f64>, alt: bool) -> [f64; 4] {
    // Per axis: the fixed point, the signed extent from it, and whether it is centred (the
    // extent is then a half-extent).
    let axis = |lo: f64, hi: f64, h: i8, d: f64| -> (f64, f64, bool) {
        let c = (lo + hi) / 2.0;
        match h {
            1 if alt => (c, hi + d - c, true),
            1 => (lo, hi + d - lo, false),
            -1 if alt => (c, lo + d - c, true),
            -1 => (hi, lo + d - hi, false),
            _ => (c, (hi - lo) / 2.0, true),
        }
    };
    let (fx, mut ex, cx) = axis(r[0], r[2], hx, d[0]);
    let (fy, mut ey, cy) = axis(r[1], r[3], hy, d[1]);
    if let Some(k) = ratio.filter(|k| k.is_finite() && *k > 0.0) {
        let full = |e: f64, c: bool| e.abs() * if c { 2.0 } else { 1.0 };
        let (w, h) = (full(ex, cx), full(ey, cy));
        let (w, h) = match (hx != 0, hy != 0) {
            // A corner: the larger relative extent wins.
            (true, true) => {
                let s = (w / k).max(h);
                (s * k, s)
            }
            (true, false) => (w, w / k),
            (false, true) => (h * k, h),
            (false, false) => (w, h),
        };
        let back = |e: f64, len: f64, c: bool| if e < 0.0 { -1.0 } else { 1.0 } * if c { len / 2.0 } else { len };
        ex = back(ex, w, cx);
        ey = back(ey, h, cy);
    }
    let span = |f: f64, e: f64, c: bool| if c { (f - e, f + e) } else { (f, f + e) };
    let ((ax, bx), (ay, by)) = (span(fx, ex, cx), span(fy, ey, cy));
    normalized([ax, ay], [bx, by])
}

/// Space is held (set by the canvas each frame, or by `ui.pointer`'s `space` flag).
pub fn set_space(app: &mut PhotocraftApp, down: bool) {
    app.crop.space = down;
}

/// As in Photoshop, the Crop tool always shows a frame: on picking the tool, and after a crop is
/// cancelled or committed, it frames the selection's bounds, or the whole canvas.
pub fn ensure_frame(app: &mut PhotocraftApp) {
    if app.ui.tool != Tool::Crop {
        app.crop.editing = false;
        // Leaving the tool drops an untouched default frame (a drawn one stays pending).
        if app.crop.default_frame {
            app.ui.crop_rect = None;
            app.crop.default_frame = false;
        }
        return;
    }
    let key = app.session.active_index().zip(app.session.active()).map(|(i, st)| (i, st.doc.size.width, st.doc.size.height));
    // Another document (or a resized one) gets its own default frame.
    if app.crop.default_frame && app.crop.frame_for != key && app.crop.drag.is_none() {
        app.ui.crop_rect = None;
    }
    if app.ui.crop_rect.is_some() || app.crop.drag.is_some() {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let canvas = st.doc.bounds();
    // Computed once per new frame, not per frame drawn.
    let r = st.doc.selection.as_ref().map(|s| s.content_bounds().intersect(&canvas)).filter(|b| !b.is_empty()).unwrap_or(canvas);
    if r.is_empty() {
        return;
    }
    app.ui.crop_rect = Some([f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)]);
    app.crop.default_frame = true;
    app.crop.frame_for = key;
    app.crop.editing = false;
    claim(app);
}

/// A crop gesture is in progress: Space repositions the frame rather than panning.
pub fn active(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.crop.drag.is_some()
}

/// Cancel the pending crop: the frame, any gesture on it and the default-frame state. With the
/// Crop tool still picked, [`ensure_frame`] then makes the active document a new default frame.
pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.crop_rect = None;
    app.crop.drag = None;
    app.crop.default_frame = false;
    app.crop.frame_for = None;
    app.crop.editing = false;
}

/// The pending frame belongs to the active document.
fn claim(app: &mut PhotocraftApp) {
    app.crop.doc = app.session.active().map(|st| st.doc.id);
}

/// A crop is pending: the Crop tool's frame on the active document has been set (drawn, moved,
/// resized or turned), not just the untouched default frame the tool shows.
pub fn pending(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop
        && app.ui.crop_rect.is_some()
        && !app.crop.default_frame
        && (app.crop.doc.is_none() || app.crop.doc == app.session.active().map(|st| st.doc.id))
}

/// Menu commands Photoshop greys out while a crop is pending (#1918), until it is committed or
/// cancelled. Checked item by item in Photoshop 27.11: everything except Close, Save, Save As,
/// Save a Copy; Undo, Redo, Toggle Last State, Search; Image › Crop (which commits the frame); the
/// View menu but Proof Setup, Pixel Aspect Ratio (Correction), 32-bit Preview Options, Flip
/// Horizontal and Screen Mode; the Window menu but Workspace and Adjustments; Help but System Info.
pub fn blocks(app: &PhotocraftApp, id: &str) -> bool {
    pending(app) && !available_while_pending(id)
}

fn available_while_pending(id: &str) -> bool {
    if let Some(v) = id.strip_prefix("view.") {
        let greyed = ["proofSetup.", "pixelAspectRatio", "thirtyTwoBitPreviewOptions", "flipHorizontal", "screenMode."];
        return !greyed.iter().any(|g| v.starts_with(g));
    }
    if let Some(w) = id.strip_prefix("window.") {
        return !(w.starts_with("workspace.") || w == "panel.adjustments");
    }
    if id.starts_with("help.") {
        return id != "help.systemInfo";
    }
    matches!(
        id,
        "file.close"
            | "file.save"
            | "file.saveAs"
            | "file.saveACopy"
            | "file.exit"
            | "edit.undo"
            | "edit.redo"
            | "edit.toggleLastState"
            | "edit.search"
            | "image.crop"
    )
}

/// A pending crop belongs to the document it was drawn on (#1918): when another document becomes
/// active (File › New, Open, a tab switch, closing the document) it is cancelled, so it never
/// shows on or crops another document. (As in Photoshop: switching to another document discards
/// the pending crop, and that document gets the tool's own frame.) Run on every frame and command
/// (`sync_views`) and before a commit.
pub fn cancel_stale(app: &mut PhotocraftApp) {
    let active = app.session.active().map(|st| st.doc.id);
    let pending = app.ui.crop_rect.is_some() || app.crop.drag.is_some();
    // A frame made before any document was claimed (control channel, tests) is the active one's.
    if pending && app.crop.doc.is_some() && app.crop.doc != active {
        cancel(app);
    }
    app.crop.doc = active;
}

/// The frame is being edited, so the canvas shows the pixels past its edges (Photoshop's crop
/// preview): from the first press with the tool until the crop is committed or cancelled.
pub fn shows_beyond_canvas(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::Crop && app.ui.crop_rect.is_some() && (app.crop.editing || app.crop.drag.is_some())
}

/// The button went down on the canvas with the Crop tool: the frame is being edited from now on.
/// Returns true when that is new.
pub fn press(app: &mut PhotocraftApp) -> bool {
    let new = app.ui.tool == Tool::Crop && app.ui.crop_rect.is_some() && !app.crop.editing;
    if new {
        app.crop.editing = true;
    }
    new
}

fn tolerance(app: &PhotocraftApp) -> f64 {
    HANDLE_PX / (app.current_zoom() as f64).max(0.01)
}

/// Pointer input for the Crop tool. Returns true when the event was its.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    if app.ui.tool != Tool::Crop {
        app.crop.drag = None;
        return false;
    }
    let p = match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    if app.session.active().is_none() || !p[0].is_finite() || !p[1].is_finite() {
        return true;
    }
    match ev {
        ToolEvent::Down { .. } => {
            claim(app);
            app.crop.editing = true;
            let frame = app.ui.crop_rect.filter(|r| r.iter().all(|v| v.is_finite()));
            app.crop.drag = Some(match frame.map(|r| (r, hit(r, p, tolerance(app)))) {
                Some((rect, Hit::Handle(hx, hy))) => CropDrag::Resize { hx, hy, start: p, rect },
                Some((rect, Hit::Inside)) if !app.crop.default_frame => CropDrag::Move { start: p, rect },
                _ => CropDrag::Draw { anchor: p, cur: p, last: p, prev: app.ui.crop_rect },
            });
        }
        ToolEvent::Move { .. } => update(app, p, mods),
        ToolEvent::Up { .. } => {
            update(app, p, mods);
            let Some(drag) = app.crop.drag.take() else { return true };
            let ok = app.ui.crop_rect.is_some_and(size_ok);
            if ok {
                app.crop.default_frame = false;
            }
            match drag {
                CropDrag::Draw { prev, .. } if !ok => app.ui.crop_rect = prev,
                CropDrag::Move { rect, .. } | CropDrag::Resize { rect, .. } if !ok => app.ui.crop_rect = Some(rect),
                _ => {}
            }
        }
    }
    true
}

/// The frame follows the pointer at `p`.
fn update(app: &mut PhotocraftApp, p: [f64; 2], mods: Modifiers) {
    let ratio = preset_ratio(app);
    let space = app.crop.space;
    let Some(drag) = app.crop.drag.as_mut() else { return };
    let rect = match drag {
        CropDrag::Draw { anchor, cur, last, .. } => {
            if space {
                // Space: the frame being drawn follows the pointer, keeping its size.
                let d = [p[0] - last[0], p[1] - last[1]];
                *anchor = [anchor[0] + d[0], anchor[1] + d[1]];
                *cur = [cur[0] + d[0], cur[1] + d[1]];
            } else {
                *cur = p;
            }
            *last = p;
            if *anchor == *cur {
                return;
            }
            drawn(*anchor, *cur, ratio, mods)
        }
        CropDrag::Move { start, rect } => {
            let (dx, dy) = (p[0] - start[0], p[1] - start[1]);
            [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy]
        }
        CropDrag::Resize { hx, hy, start, rect } => {
            let keep = || {
                let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
                (w > 0.0 && h > 0.0).then(|| w / h)
            };
            let ratio = ratio.or_else(|| if mods.shift { keep() } else { None });
            resized(*rect, *hx, *hy, [p[0] - start[0], p[1] - start[1]], ratio, mods.alt)
        }
    };
    app.ui.crop_rect = Some(rect);
}

/// Cursor over the canvas at document point `p`: resize arrows on the edges, move inside.
pub fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<CursorIcon> {
    if app.ui.tool != Tool::Crop {
        return None;
    }
    let h = match app.crop.drag {
        Some(CropDrag::Move { .. }) => Hit::Inside,
        Some(CropDrag::Resize { hx, hy, .. }) => Hit::Handle(hx, hy),
        Some(CropDrag::Draw { .. }) => Hit::Outside,
        None => hit(app.ui.crop_rect?, p, tolerance(app)),
    };
    Some(match h {
        Hit::Handle(-1, -1) | Hit::Handle(1, 1) => CursorIcon::ResizeNwSe,
        Hit::Handle(-1, 1) | Hit::Handle(1, -1) => CursorIcon::ResizeNeSw,
        Hit::Handle(_, 0) => CursorIcon::ResizeHorizontal,
        Hit::Handle(..) => CursorIcon::ResizeVertical,
        Hit::Inside => CursorIcon::Move,
        Hit::Outside => CursorIcon::Crosshair,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::tool_event;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
    use serde_json::json;

    const NONE: Modifiers = Modifiers::NONE;
    const SHIFT: Modifiers = Modifiers::SHIFT;

    fn app(depth: SampleType) -> PhotocraftApp {
        let doc = Document::with_background("crop", Size::new(200, 100), ColorMode::Rgb, depth, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::Crop;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app
    }

    fn drag(app: &mut PhotocraftApp, pts: &[[f64; 2]], mods: Modifiers) {
        let (first, last) = (pts[0], pts[pts.len() - 1]);
        tool_event(app, ToolEvent::Down { x: first[0], y: first[1], pressure: 1.0 }, mods);
        for p in &pts[1..] {
            tool_event(app, ToolEvent::Move { x: p[0], y: p[1], pressure: 1.0 }, mods);
        }
        tool_event(app, ToolEvent::Up { x: last[0], y: last[1] }, mods);
    }

    #[test]
    fn picking_the_tool_frames_the_canvas_or_the_selection() {
        // #668: the Crop tool started with no frame.
        let mut app = app(SampleType::U8);
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 200.0, 100.0]));
        // Committing the untouched frame crops nothing.
        let steps = app.session.active().unwrap().history.past_len();
        crate::canvas::commit_crop(&mut app);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(200, 100));
        // A drag inside the untouched frame draws a new one instead of moving it.
        ensure_frame(&mut app);
        drag(&mut app, &[[50.0, 20.0], [80.0, 40.0], [120.0, 70.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([50.0, 20.0, 120.0, 70.0]));
        // Now it's a real frame: a drag inside moves it.
        drag(&mut app, &[[60.0, 30.0], [70.0, 30.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([60.0, 20.0, 130.0, 70.0]));
        // Cancelling frames the selection's bounds when there is one.
        app.run("select.rect", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        app.ui.crop_rect = None;
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([10.0, 10.0, 40.0, 30.0]));
        // Leaving the tool drops the untouched frame.
        app.ui.tool = Tool::Brush;
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, None);
    }

    #[test]
    fn the_frame_is_edited_from_a_press_until_commit_cancel_or_another_tool() {
        let mut app = app(SampleType::U8);
        ensure_frame(&mut app);
        assert!(!shows_beyond_canvas(&app), "a new frame is not being edited");
        assert!(press(&mut app));
        assert!(shows_beyond_canvas(&app));
        assert!(!press(&mut app), "only the first press is news");
        // A click on the frame keeps it edited.
        drag(&mut app, &[[50.0, 50.0]], NONE);
        assert!(shows_beyond_canvas(&app));
        // Esc drops the frame: the new default one isn't edited.
        app.ui.crop_rect = None;
        ensure_frame(&mut app);
        assert!(!shows_beyond_canvas(&app));
        // A drawn frame is edited until it is committed.
        drag(&mut app, &[[10.0, 10.0], [60.0, 40.0]], NONE);
        assert!(shows_beyond_canvas(&app));
        crate::canvas::commit_crop(&mut app);
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 50.0, 30.0]));
        assert!(!shows_beyond_canvas(&app));
        // Or until another tool is picked.
        drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
        app.ui.tool = Tool::Brush;
        ensure_frame(&mut app);
        assert!(!app.crop.editing && !shows_beyond_canvas(&app));
        assert!(!press(&mut app), "no frame without the Crop tool");
    }

    #[test]
    fn hit_test_finds_handles_inside_and_outside() {
        let r = [10.0, 10.0, 110.0, 60.0];
        assert_eq!(hit(r, [10.0, 10.0], 4.0), Hit::Handle(-1, -1));
        assert_eq!(hit(r, [112.0, 61.0], 4.0), Hit::Handle(1, 1));
        assert_eq!(hit(r, [60.0, 8.0], 4.0), Hit::Handle(0, -1));
        assert_eq!(hit(r, [108.0, 30.0], 4.0), Hit::Handle(1, 0));
        assert_eq!(hit(r, [60.0, 30.0], 4.0), Hit::Inside);
        assert_eq!(hit(r, [150.0, 30.0], 4.0), Hit::Outside);
        // A tiny frame keeps a movable middle.
        assert_eq!(hit([0.0, 0.0, 4.0, 4.0], [2.0, 2.0], 8.0), Hit::Inside);
    }

    #[test]
    fn dragging_inside_moves_the_frame() {
        let mut app = app(SampleType::U8);
        drag(&mut app, &[[20.0, 20.0], [80.0, 60.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 80.0, 60.0]));
        drag(&mut app, &[[50.0, 40.0], [60.0, 45.0], [70.0, 50.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([40.0, 30.0, 100.0, 70.0]), "moved, not redrawn");
        // Outside, a drag still draws a new frame.
        drag(&mut app, &[[150.0, 10.0], [190.0, 90.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([150.0, 10.0, 190.0, 90.0]));
        // A click outside keeps the frame.
        drag(&mut app, &[[5.0, 5.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([150.0, 10.0, 190.0, 90.0]));
    }

    #[test]
    fn moving_the_frame_snaps_its_edges() {
        let mut app = app(SampleType::U8);
        app.ui.extras.snap = true;
        app.ui.crop_rect = Some([10.0, 10.0, 50.0, 40.0]);
        drag(&mut app, &[[30.0, 25.0], [23.0, 25.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([0.0, 10.0, 40.0, 40.0]), "left edge snaps to the document edge");
    }

    #[test]
    fn handles_resize_with_shift_and_alt() {
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
        // Right edge.
        drag(&mut app, &[[100.0, 40.0], [120.0, 50.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 120.0, 60.0]));
        // Bottom-right corner, free.
        drag(&mut app, &[[120.0, 60.0], [140.0, 70.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 140.0, 70.0]));
        // ⇧ keeps 120:50 from the top-left corner: the larger relative extent wins.
        app.ui.crop_rect = Some([20.0, 20.0, 140.0, 70.0]);
        drag(&mut app, &[[20.0, 20.0], [10.0, 0.0]], SHIFT);
        let r = app.ui.crop_rect.unwrap();
        assert_eq!((r[2], r[3]), (140.0, 70.0), "opposite corner fixed");
        assert!(((r[2] - r[0]) / (r[3] - r[1]) - 120.0 / 50.0).abs() < 1e-9, "{r:?}");
        assert_eq!(r[3] - r[1], 70.0);
        // ⌥ resizes about the centre.
        app.ui.crop_rect = Some([40.0, 20.0, 80.0, 60.0]);
        drag(&mut app, &[[80.0, 40.0], [90.0, 40.0]], Modifiers::ALT);
        assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 90.0, 60.0]));
        // An options-bar ratio holds on an edge drag, centred on the other axis.
        app.ui.tool_options.crop_ratio = "1:1".into();
        app.ui.crop_rect = Some([40.0, 20.0, 80.0, 60.0]);
        drag(&mut app, &[[80.0, 40.0], [100.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([40.0, 10.0, 100.0, 70.0]));
    }

    #[test]
    fn dragging_a_handle_past_the_other_side_flips_and_collapse_reverts() {
        let mut app = app(SampleType::U8);
        app.ui.crop_rect = Some([20.0, 20.0, 60.0, 60.0]);
        drag(&mut app, &[[60.0, 40.0], [10.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 20.0, 60.0]));
        // Collapsing to nothing restores the frame the gesture started from.
        drag(&mut app, &[[20.0, 40.0], [10.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([10.0, 20.0, 20.0, 60.0]));
    }

    #[test]
    fn space_repositions_the_frame_being_drawn() {
        let mut app = app(SampleType::U8);
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, NONE);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
        assert!(active(&app));
        set_space(&mut app, true);
        tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 50.0, pressure: 1.0 }, NONE);
        assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 70.0, 50.0]), "same size, moved");
        set_space(&mut app, false);
        tool_event(&mut app, ToolEvent::Move { x: 90.0, y: 60.0, pressure: 1.0 }, NONE);
        tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 60.0 }, NONE);
        assert_eq!(app.ui.crop_rect, Some([30.0, 20.0, 90.0, 60.0]), "sizing resumes from the moved anchor");
        assert!(!active(&app));
    }

    /// Real egui input on the canvas: holding Space mid-draw moves the frame instead of panning.
    #[test]
    fn space_on_the_canvas_moves_the_frame_not_the_view() {
        use egui::{Event, Key, PointerButton, pos2};
        let mut a = app(SampleType::U8);
        let view = crate::state::View { zoom: 2.0, center: [100.0, 50.0], fit_pending: false, fill_pending: false, doc_size: [200, 100], rotation: 0.0 };
        a.ui.views = vec![view.clone()];
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let v = app.ui.views[0].clone();
                crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
            },
            a,
        );
        h.run_steps(2);
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        let p0 = pos2(200.0, 150.0);
        h.event(Event::PointerMoved(p0));
        h.event(button(p0, true));
        h.step();
        for p in [pos2(220.0, 170.0), pos2(260.0, 190.0)] {
            h.event(Event::PointerMoved(p));
            h.step();
        }
        let drawn = h.state().ui.crop_rect.unwrap();
        // The frame starts where the button went down (200, 150), not at the first move past
        // egui's drag threshold (#123).
        assert_eq!((drawn[2] - drawn[0], drawn[3] - drawn[1]), (30.0, 20.0), "60x40 screen px at 200%");
        h.event(space(true));
        h.step();
        for p in [pos2(280.0, 200.0), pos2(300.0, 210.0)] {
            h.event(Event::PointerMoved(p));
            h.step();
        }
        let moved = h.state().ui.crop_rect.unwrap();
        assert_eq!(moved, [drawn[0] + 20.0, drawn[1] + 10.0, drawn[2] + 20.0, drawn[3] + 10.0], "moved by 40x20 screen px");
        h.event(space(false));
        h.event(button(pos2(300.0, 210.0), false));
        h.step();
        let st = h.state();
        assert_eq!(st.ui.crop_rect, Some(moved));
        assert_eq!(st.ui.views[0].center, view.center, "the view did not pan");
        assert!(st.crop.drag.is_none());
    }

    /// #1918: a pending crop belongs to the document it was drawn on. File › New, opening a file
    /// or switching tabs cancels it; it never shows on, or crops, another document.
    #[test]
    fn a_pending_crop_stays_with_its_document() {
        let size = |app: &PhotocraftApp| app.session.active().map(|st| st.doc.size);
        // File › New.
        let mut app = app(SampleType::U8);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        assert_eq!(app.ui.crop_rect, Some([10.0, 10.0, 50.0, 40.0]));
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        assert_eq!(app.ui.crop_rect, None, "the first document's frame is gone");
        assert!(app.crop.drag.is_none() && !app.crop.editing && !app.crop.default_frame);
        crate::canvas::commit_crop(&mut app);
        assert_eq!(size(&app), Some(Size::new(300, 150)), "↵ crops nothing");
        // The new document gets its own default frame.
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, Some([0.0, 0.0, 300.0, 150.0]));
        assert!(app.crop.default_frame);
        // Switching tabs (outside a command), even before the next frame's sync: ↵ never crops
        // the other document.
        drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
        assert!(app.session.set_active(0));
        crate::canvas::commit_crop(&mut app);
        assert_eq!(size(&app), Some(Size::new(200, 100)));
        assert_eq!(app.session.documents()[1].doc.size, Size::new(300, 150));
        assert_eq!(app.ui.crop_rect, None);
        // The next frame's sync also drops a frame left on a tab switched away from.
        drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
        assert!(app.session.set_active(1));
        app.sync_views();
        assert_eq!(app.ui.crop_rect, None);
        // Closing another document keeps the pending crop; closing its own cancels it.
        drag(&mut app, &[[20.0, 20.0], [60.0, 60.0]], NONE);
        app.session.close(0);
        app.sync_views();
        assert_eq!(app.ui.crop_rect, Some([20.0, 20.0, 60.0, 60.0]), "still its document");
        crate::canvas::commit_crop(&mut app);
        assert_eq!(size(&app), Some(Size::new(40, 40)));
        drag(&mut app, &[[5.0, 5.0], [25.0, 25.0]], NONE);
        app.session.close(0);
        app.sync_views();
        assert!(app.session.active().is_none());
        assert_eq!(app.ui.crop_rect, None);
        assert!(app.crop.drag.is_none());
        crate::canvas::commit_crop(&mut app);
        ensure_frame(&mut app);
        assert_eq!(app.ui.crop_rect, None, "no document, no frame");
    }

    /// A drag in progress when its document goes away is dropped too, not finished elsewhere.
    #[test]
    fn a_crop_drag_ends_with_its_document() {
        let mut app = app(SampleType::U8);
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, NONE);
        tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 40.0, pressure: 1.0 }, NONE);
        app.run("file.new", json!({"width": 300, "height": 150})).unwrap();
        tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 50.0 }, NONE);
        assert_eq!(app.ui.crop_rect, None);
        crate::canvas::commit_crop(&mut app);
        assert_eq!(app.session.active().map(|st| st.doc.size), Some(Size::new(300, 150)));
    }

    /// #1918: as in Photoshop, a pending crop (not the untouched default frame) greys almost every
    /// menu command; the ones Photoshop keeps stay available, and commit or cancel frees the rest.
    /// Walks the whole menu catalog.
    #[test]
    fn a_pending_crop_greys_the_menus_photoshop_greys() {
        let mut app = app(SampleType::U8);
        ensure_frame(&mut app);
        assert!(app.crop.default_frame);
        let ids: Vec<&str> = crate::menu_catalog::CATALOG.iter().map(|e| e.3).filter(|id| *id != "---").collect();
        for id in &ids {
            assert!(!blocks(&app, id), "default frame: {id} stays as it was");
        }
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        assert!(pending(&app));
        let kept = [
            "file.close",
            "file.save",
            "file.saveAs",
            "file.saveACopy",
            "edit.undo",
            "edit.redo",
            "edit.toggleLastState",
            "edit.search",
            "image.crop",
            "view.proofColors",
            "view.gamutWarning",
            "view.zoomIn",
            "view.zoomOut",
            "view.fitOnScreen",
            "view.actualPixels",
            "view.twoHundredPercent",
            "view.printSize",
            "view.extras",
            "view.show.grid",
            "view.rulers",
            "view.snap",
            "view.snapTo.guides",
            "view.lockSlices",
            "view.newGuide",
            "window.arrange.tile",
            "window.panel.layers",
            "help.about",
        ];
        let greyed = [
            "file.new",
            "file.open",
            "file.openAs",
            "file.closeAll",
            "file.revert",
            "file.export.exportAs",
            "file.placeEmbedded",
            "file.automate.batch",
            "file.scripts.browse",
            "file.import.notes",
            "file.fileInfo",
            "file.print",
            "edit.cut",
            "edit.fill",
            "edit.freeTransform",
            "edit.colorSettings",
            "edit.preferences.general",
            "image.imageSize",
            "image.mode.grayscale",
            "layer.new.layer",
            "select.all",
            "filter.blur.gaussianBlur",
            "view.proofSetup.workingCmyk",
            "view.pixelAspectRatio.square",
            "view.pixelAspectRatioCorrection",
            "view.thirtyTwoBitPreviewOptions",
            "view.flipHorizontal",
            "view.screenMode.fullScreen",
            "window.workspace.essentials",
            "window.panel.adjustments",
            "help.systemInfo",
        ];
        for id in kept {
            assert!(!blocks(&app, id), "{id} stays available");
        }
        for id in greyed {
            assert!(blocks(&app, id), "{id} is greyed");
            assert!(!crate::menus::is_enabled(&app, id), "{id} greyed in the menus");
            assert!(!crate::menus::modal_allows(&app, id), "{id} refused over the menu gate");
        }
        assert!(
            ids.iter()
                .filter(|id| id.starts_with("layer.") || id.starts_with("type.") || id.starts_with("select.") || id.starts_with("filter."))
                .all(|id| blocks(&app, id))
        );
        // Committing still runs `image.crop` itself, and frees the menus again.
        crate::canvas::commit_crop(&mut app);
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(40, 30));
        assert!(!pending(&app) && crate::menus::is_enabled(&app, "file.new"));
        // Cancelling does too.
        drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
        assert!(blocks(&app, "file.new"));
        cancel(&mut app);
        assert!(!blocks(&app, "file.new"));
        // Another tool: nothing is pending.
        drag(&mut app, &[[5.0, 5.0], [20.0, 20.0]], NONE);
        app.ui.tool = Tool::Brush;
        assert!(!blocks(&app, "file.new"));
    }

    /// Image › Crop with a pending crop commits the frame (the one Image item Photoshop keeps).
    #[test]
    fn image_crop_menu_commits_a_pending_crop() {
        let mut app = app(SampleType::U8);
        drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "image.crop", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.size, Size::new(40, 30));
        assert!(app.ui.crop_rect.is_none() || app.crop.default_frame);
    }

    #[test]
    fn commit_crops_to_the_moved_frame_at_8_and_16_bit() {
        for depth in [SampleType::U8, SampleType::U16] {
            let mut app = app(depth);
            app.run("select.rect", json!({"x": 60, "y": 40, "width": 1, "height": 1})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            drag(&mut app, &[[10.0, 10.0], [50.0, 40.0]], NONE);
            drag(&mut app, &[[30.0, 25.0], [70.0, 45.0]], NONE);
            assert_eq!(app.ui.crop_rect, Some([50.0, 30.0, 90.0, 60.0]));
            crate::canvas::commit_crop(&mut app);
            assert!(app.ui.crop_rect.is_none());
            let doc = &app.session.active().unwrap().doc;
            assert_eq!((doc.size.width, doc.size.height), (40, 30), "{depth:?}");
            assert_eq!(doc.depth, depth);
            let px = doc.layers[0].surface().unwrap().rgba(10, 10);
            assert!(px[0] > 0.99 && px[1] < 0.01, "{depth:?} {px:?}");
        }
    }

    #[test]
    fn degenerate_and_bad_input_never_panics() {
        let mut app = app(SampleType::U8);
        for r in [[0.0; 4], [5.0, 5.0, 5.0, 5.0], [10.0, 10.0, 0.0, 0.0], [f64::NAN, 0.0, 10.0, 10.0], [f64::INFINITY, 0.0, f64::MAX, 1e300]] {
            app.ui.crop_rect = Some(r);
            for m in [NONE, SHIFT, Modifiers::ALT, SHIFT | Modifiers::ALT] {
                drag(&mut app, &[[5.0, 5.0], [f64::NAN, 3.0], [1e300, -1e300], [6.0, 7.0]], m);
                let _ = cursor(&app, [5.0, 5.0]);
                app.ui.crop_rect = Some(r);
            }
        }
        app.ui.tool_options.crop_ratio = "0:0".into();
        app.ui.crop_rect = Some([0.0, 0.0, 10.0, 10.0]);
        drag(&mut app, &[[10.0, 10.0], [30.0, 30.0]], SHIFT);
        crate::canvas::commit_crop(&mut app);
        // No document: events are swallowed quietly.
        let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        empty.ui.tool = Tool::Crop;
        drag(&mut empty, &[[1.0, 1.0], [9.0, 9.0]], NONE);
        assert!(empty.ui.crop_rect.is_none());
    }

    #[test]
    fn cursor_follows_the_frame() {
        let mut app = app(SampleType::U8);
        assert_eq!(cursor(&app, [5.0, 5.0]), None, "no frame yet: the default crosshair");
        app.ui.crop_rect = Some([20.0, 20.0, 100.0, 60.0]);
        assert_eq!(cursor(&app, [20.0, 20.0]), Some(CursorIcon::ResizeNwSe));
        assert_eq!(cursor(&app, [100.0, 20.0]), Some(CursorIcon::ResizeNeSw));
        assert_eq!(cursor(&app, [60.0, 60.0]), Some(CursorIcon::ResizeVertical));
        assert_eq!(cursor(&app, [60.0, 40.0]), Some(CursorIcon::Move));
        assert_eq!(cursor(&app, [150.0, 40.0]), Some(CursorIcon::Crosshair));
        app.ui.tool = Tool::Brush;
        assert_eq!(cursor(&app, [60.0, 40.0]), None);
    }
}
