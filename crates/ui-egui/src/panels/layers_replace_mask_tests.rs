//! #16787: a layer mask dragged (or Alt-dragged) onto a layer that already has a layer mask asks
//! "Replace Layer Mask?" with Yes (the default), No and an unticked "Don't show again", as in the
//! reference application, instead of refusing the drop. Yes moves (with Alt copies) the mask in one
//! undo step, No changes nothing, a ticked "Don't show again" replaces at once next time. A drop on
//! the mask's own layer, a vector mask's drop, and a drop on a group or adjustment layer that has a
//! mask (not measured) are still refused.

use craft::layers::ui::tree::LayerPart;
use craft::ui::dialog::{DialogId, StackDialog as _, StandardPrompt};
use egui::Modifiers;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::LayerMask;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::dialogs::alerts;
use crate::dialogs::replace_layer_mask_prompt::{PREF, ReplaceMask};
use crate::layers_panel_nodes::{drag, layer_id, name_point, thumb};

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness(s: Session) -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    h
}

/// 64 x 48 with "Source" (a mask revealing 8,8 24x16) under "Target" (a mask hiding all).
fn masked_pair() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Source"})).unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 24, "height": 16})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Target"})).unwrap();
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    s
}

fn mask(h: &H, name: &str) -> Option<LayerMask> {
    h.state().session.active().unwrap().doc.layer(layer_id(h.state(), name)).unwrap().mask.clone()
}

fn steps(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn drag_mask(h: &mut H, from: &str, to: &str, mods: Modifiers) {
    let at = thumb(h, from, LayerPart::LayerMask).center();
    let target = name_point(h, to);
    drag(h, at, target, mods);
}

fn question(h: &H) -> Option<DialogId> {
    alerts::prompt_tagged::<ReplaceMask>(h.state())
}

/// Presses the question's default button, ticking "Don't show again" when `skip`.
fn answer_default(h: &mut H, id: DialogId, skip: bool) {
    let ctx = h.ctx.clone();
    if skip {
        h.state_mut().framework.dialogs.set_json_field(&ctx, id, "skip", json!(true)).unwrap();
    }
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    h.run_steps(3);
}

fn answer_no(h: &mut H, id: DialogId) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "choice", json!("cancel")).unwrap();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    h.run_steps(3);
}

#[test]
fn a_drop_on_a_masked_layer_asks_replace_layer_mask_with_yes_no_and_an_unticked_box() {
    for mods in [Modifiers::NONE, Modifiers::ALT] {
        let mut h = harness(masked_pair());
        let (source, target, n) = (mask(&h, "Source"), mask(&h, "Target"), steps(&h));
        drag_mask(&mut h, "Source", "Target", mods);
        assert!(question(&h).is_some(), "{mods:?}: the drop asks");
        let prompt = h.state().framework.dialogs.get::<StandardPrompt>().unwrap();
        assert_eq!(prompt.json_fields().unwrap()["skip"], json!(false), "{mods:?}: Don't show again starts unticked");
        h.run_steps(3);
        for label in ["Replace Layer Mask?", "Yes", "No", "Don't show again"] {
            assert!(h.query_by_label(label).is_some(), "{mods:?}: the alert shows {label:?}");
        }
        assert_eq!((mask(&h, "Source"), mask(&h, "Target"), steps(&h)), (source, target, n), "{mods:?}: nothing changes before the answer");
    }
}

#[test]
fn no_leaves_both_layers_and_the_history_alone() {
    for mods in [Modifiers::NONE, Modifiers::ALT] {
        let mut h = harness(masked_pair());
        let (source, target, n) = (mask(&h, "Source"), mask(&h, "Target"), steps(&h));
        drag_mask(&mut h, "Source", "Target", mods);
        let id = question(&h).expect("the question");
        answer_no(&mut h, id);
        assert!(question(&h).is_none());
        assert_eq!((mask(&h, "Source"), mask(&h, "Target"), steps(&h)), (source, target, n), "{mods:?}");
    }
}

#[test]
fn yes_the_default_moves_the_mask_in_one_undo_step() {
    let mut h = harness(masked_pair());
    let (source, target, n) = (mask(&h, "Source"), mask(&h, "Target"), steps(&h));
    assert_ne!(source, target);
    drag_mask(&mut h, "Source", "Target", Modifiers::NONE);
    let id = question(&h).expect("the question");
    answer_default(&mut h, id, false);
    assert_eq!(mask(&h, "Source"), None, "the mask left its layer");
    assert_eq!(mask(&h, "Target").map(|m| m.surface), source.clone().map(|m| m.surface), "and replaced the target's");
    assert_eq!(steps(&h), n + 1, "one step");
    assert!(h.state_mut().session.undo());
    assert_eq!((mask(&h, "Source"), mask(&h, "Target")), (source, target), "one undo puts both back");
}

#[test]
fn yes_after_an_alt_drag_copies_the_mask_in_one_undo_step() {
    let mut h = harness(masked_pair());
    let (source, target, n) = (mask(&h, "Source"), mask(&h, "Target"), steps(&h));
    drag_mask(&mut h, "Source", "Target", Modifiers::ALT);
    let id = question(&h).expect("the question");
    answer_default(&mut h, id, false);
    assert_eq!(mask(&h, "Source"), source, "the original stays");
    assert_eq!(mask(&h, "Target").map(|m| m.surface), source.clone().map(|m| m.surface), "a copy replaced the target's");
    assert_eq!(steps(&h), n + 1, "one step");
    assert!(h.state_mut().session.undo());
    assert_eq!((mask(&h, "Source"), mask(&h, "Target")), (source, target));
}

#[test]
fn a_ticked_dont_show_again_replaces_at_once_next_time() {
    let mut h = harness(masked_pair());
    let source = mask(&h, "Source");
    drag_mask(&mut h, "Source", "Target", Modifiers::ALT);
    let id = question(&h).expect("the question");
    answer_default(&mut h, id, true);
    assert!(alerts::dismissed(h.state(), PREF), "the box is remembered");
    // Give Target its own mask again, then drop once more: no question, the mask is replaced.
    let target = layer_id(h.state(), "Target").0;
    h.state_mut().run("layer.layerMask.delete", json!({"layer": target})).unwrap();
    h.state_mut().run("layer.layerMask.hideAll", json!({"layer": target})).unwrap();
    h.run_steps(3);
    let n = steps(&h);
    drag_mask(&mut h, "Source", "Target", Modifiers::NONE);
    assert!(question(&h).is_none(), "no question");
    assert_eq!(mask(&h, "Source"), None, "the mask moved");
    assert_eq!(mask(&h, "Target").map(|m| m.surface), source.map(|m| m.surface));
    assert_eq!(steps(&h), n + 1, "in one step");
}

#[test]
fn an_edit_before_yes_replaces_nothing() {
    let mut h = harness(masked_pair());
    drag_mask(&mut h, "Source", "Target", Modifiers::NONE);
    let id = question(&h).expect("the question");
    let target = layer_id(h.state(), "Target").0;
    h.state_mut().run("layer.layerMask.enabled", json!({"layer": target, "enabled": false})).unwrap();
    let (source, masked) = (mask(&h, "Source"), mask(&h, "Target"));
    answer_default(&mut h, id, false);
    assert_eq!((mask(&h, "Source"), mask(&h, "Target")), (source, masked), "the stale answer changes nothing");
    assert!(h.state().ui.status_error, "and says why");
}

#[test]
fn a_drop_on_a_layer_without_a_mask_still_moves_without_asking() {
    let mut s = masked_pair();
    s.execute("layer.new.layer", json!({"name": "Plain"})).unwrap();
    let mut h = harness(s);
    let (source, n) = (mask(&h, "Source"), steps(&h));
    drag_mask(&mut h, "Source", "Plain", Modifiers::NONE);
    assert!(question(&h).is_none());
    assert_eq!(mask(&h, "Plain"), source);
    assert_eq!(steps(&h), n + 1);
}

#[test]
fn the_own_layer_a_vector_mask_a_masked_group_and_a_masked_adjustment_still_refuse() {
    let mut s = masked_pair();
    s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
    s.execute("layer.newAdjustmentLayer.brightnessContrast", json!({"name": "Tone", "brightness": 12})).unwrap();
    s.execute("layer.new.group", json!({"name": "Folder"})).unwrap();
    s.execute("layer.layerMask.hideAll", json!({})).unwrap();
    let mut h = harness(s);
    assert!(mask(&h, "Tone").is_some() && mask(&h, "Folder").is_some(), "both have a mask");
    let snapshot = |h: &H| ["Source", "Target", "Tone", "Folder"].map(|name| mask(h, name));
    let (before, n) = (snapshot(&h), steps(&h));
    for to in ["Source", "Tone", "Folder"] {
        drag_mask(&mut h, "Source", to, Modifiers::NONE);
        assert!(question(&h).is_none(), "onto {to}: no question");
    }
    let vector = thumb(&h, "Target", LayerPart::VectorMask).center();
    let source = name_point(&h, "Source");
    drag(&mut h, vector, source, Modifiers::NONE);
    assert!(question(&h).is_none(), "a vector mask: no question");
    assert_eq!((snapshot(&h), steps(&h)), (before, n), "nothing changed");
}
