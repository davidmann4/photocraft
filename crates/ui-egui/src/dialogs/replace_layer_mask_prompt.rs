//! A layer mask dragged (or Alt-dragged) in the Layers panel onto a layer that already has a layer
//! mask asks "Replace Layer Mask?" first, as in Photoshop (#16787): Yes (the default) moves the mask
//! there, or copies it with Alt, replacing the mask in one undo step; No changes nothing. A ticked
//! "Don't show again" (kept with the other dialog preferences under [`PREF`]) makes later drops
//! replace without asking.
//!
//! Groups and adjustment layers that have a mask still refuse the drop: what Photoshop does there was
//! not measured.

use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;

/// Its "Don't show again", under `Preferences::dialogs`.
pub const PREF: &str = "layer.layerMask.replace";

const COMMAND: &str = "layer.layerMask.moveTo";

/// What the question asked about: the mask of `from` going to `to` (a copy when `copy`), in this
/// `revision` of `document`. An edit or another document meanwhile makes the answer replace nothing.
pub(crate) struct ReplaceMask {
    document: u64,
    revision: u64,
    from: LayerId,
    to: LayerId,
    copy: bool,
}

/// Whether a layer mask dropped on `target` replaces its mask after the question: it has a layer
/// mask and is neither a group nor an adjustment layer.
pub(crate) fn asks_before_replacing(target: &Layer) -> bool {
    target.mask.is_some() && !matches!(target.content, LayerContent::Group(_) | LayerContent::Adjustment(_))
}

/// The `layer.layerMask.moveTo` step that moves (or with `copy` copies) the mask of `from` onto
/// `to`, replacing a mask there when `replace`.
pub(crate) fn move_action(from: LayerId, to: LayerId, copy: bool, replace: bool) -> (String, Value) {
    let mut params = json!({"layer": from.0, "to": to.0, "copy": copy});
    if replace {
        params["replace"] = json!(true);
    }
    (COMMAND.into(), params)
}

/// A layer mask dropped from `from` onto `to`, which already has one. `Some` step to run at once when
/// "Don't show again" was ticked before; otherwise opens the question and returns `None`.
pub(crate) fn ask(app: &mut PhotocraftApp, from: LayerId, to: LayerId, copy: bool) -> Option<(String, Value)> {
    if alerts::dismissed(app, PREF) {
        return Some(move_action(from, to, copy, true));
    }
    let (document, revision) = app.session.active().map(|st| (st.doc.id.0, st.revision))?;
    let prompt = StandardPrompt::custom(tl!("Replace Layer Mask?").to_owned())
        .title(tl!("PhotoCraft").to_owned())
        .button(PromptChoice::Proceed, tl!("Yes").to_owned())
        .button(PromptChoice::Cancel, tl!("No").to_owned())
        .dont_show_again()
        .tag(ReplaceMask { document, revision, from, to, copy });
    // An `Err` is another question already open: the drop waits for nothing and changes nothing.
    let _ = alerts::open_prompt(app, prompt);
    None
}

/// The `PROMPTS` entry: Yes replaces the mask in one step and keeps a ticked "Don't show again"; No
/// changes nothing. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(ask) = answer.take_tag::<ReplaceMask>() else { return false };
    if answer.choice != PromptChoice::Proceed {
        return true;
    }
    match confirm(app, &ask) {
        Ok(_) if answer.skip => alerts::remember_skip(app, PREF),
        Ok(_) => {}
        Err(error) => alerts::report_error(app, error),
    }
    true
}

/// Runs the replacement when the document is still the one, in the revision, the question was asked in.
fn confirm(app: &mut PhotocraftApp, ask: &ReplaceMask) -> Result<Value, String> {
    let current = app.session.active().map(|st| (st.doc.id.0, st.revision));
    if current != Some((ask.document, ask.revision)) {
        return Err(tl!("The document changed. Drag the mask again to replace it.").into());
    }
    let (command, params) = move_action(ask.from, ask.to, ask.copy, true);
    app.run(&command, params)
}
