//! Fonts a host serves next to the app, for the platform shell that fetches them (the web build;
//! see `photocraft_text::served`): reading the manifest, the families to fetch, and installing
//! what arrived, which registers the files and re-renders the type layers drawn without them.

use std::sync::{Arc, Mutex, PoisonError};

use photocraft_doc::{LayerContent, TextLayer};
use photocraft_engine::TypeLayerRef;
pub use photocraft_text::served::{ServedFont, add_families, add_fonts, is_relative_path, parse_manifest, request_for_languages, take_requests};

use crate::PhotocraftApp;

/// Layers whose document was busy with a background job when their font arrived.
static PENDING: Mutex<Vec<TypeLayerRef>> = Mutex::new(Vec::new());

/// Registers delivered font files (`(family, bytes)`, the family as the manifest lists it) and
/// re-renders the type layers that use those families, or whose text falls back to them (a script
/// fallback, e.g. Arabic set in Inter), and show our own layout. A PSD layer still
/// showing Photoshop's pixels (not edited yet) keeps them, as it does when its font is installed
/// from the start. Call it every frame: it also retries the layers of a document that was busy.
pub fn install(app: &mut PhotocraftApp, delivered: Vec<(String, Vec<u8>)>) {
    let mut layers = std::mem::take(&mut *PENDING.lock().unwrap_or_else(PoisonError::into_inner));
    if !delivered.is_empty() {
        let families: Vec<String> = delivered.iter().map(|(f, _)| f.clone()).collect();
        // Decided before the fonts are in: "shows our own layout" compares the layer's pixels
        // with what the engine draws now, that is with the fallback font.
        pick(&app.session, &families, &mut layers);
        let mut eng = photocraft_text::shared().lock().unwrap_or_else(PoisonError::into_inner);
        for (family, bytes) in delivered {
            let added = eng.fonts.register_font_data(bytes);
            if !added.contains(&family) {
                log::warn!("a served font file listed as {family:?} registers {added:?}; the family in fonts/manifest.txt must be the font's own");
            }
        }
    }
    if !layers.is_empty() {
        let busy = app.session.refresh_type_layers(&layers);
        PENDING.lock().unwrap_or_else(PoisonError::into_inner).extend(busy);
    }
}

/// Adds to `layers` the type layers to redraw once `families` arrive, in every open document and
/// in its first History snapshot (the document as opened), which is checked on its own: since
/// then its layers may have been edited (a PSD layer re-rendered) or deleted (#6087).
fn pick(session: &photocraft_engine::Session, families: &[String], layers: &mut Vec<TypeLayerRef>) {
    for st in session.documents() {
        // While it still is the open document, the engine refreshes it with it.
        let first = st.snapshots.iter().find(|s| s.id == 0 && !Arc::ptr_eq(&s.doc, &st.doc)).map(|s| &s.doc);
        for (doc, first_snapshot) in std::iter::once((&st.doc, false)).chain(first.map(|d| (d, true))) {
            for (_, _, l) in doc.walk() {
                let key = TypeLayerRef { doc: st.doc.id, layer: l.id, first_snapshot };
                if let LayerContent::Text(t) = &l.content
                    && redraws_for(t, families)
                    && crate::type_tool::shows_own_layout(doc, t)
                    && !layers.contains(&key)
                {
                    layers.push(key);
                }
            }
        }
    }
}

/// Should the layer be redrawn when `families` arrive: it uses one of them, or its text is in a
/// script one of them is the served fallback for?
fn redraws_for(t: &TextLayer, families: &[String]) -> bool {
    uses_family(t, families) || families.iter().any(|f| photocraft_text::served::falls_back_to(&t.text, f))
}

/// Does the layer use one of `families`, by name or through the PostScript name a PSD gave?
fn uses_family(t: &TextLayer, families: &[String]) -> bool {
    t.char_runs().iter().any(|r| {
        families.contains(&r.style.font_family)
            || r.style.postscript_name.as_deref().is_some_and(|ps| families.contains(&photocraft_text::fonts::guess_from_postscript(ps).family))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_uses_a_family_by_name_or_postscript_name() {
        let t = TextLayer { text: "Hi".into(), font_family: "Open Sans".into(), ..Default::default() };
        assert!(uses_family(&t, &["Open Sans".into()]));
        assert!(!uses_family(&t, &["Lato".into()]));
        let mut psd = TextLayer { text: "Hi".into(), font_family: "Inter".into(), ..Default::default() };
        psd.runs = vec![photocraft_doc::text::TextRun {
            len: 2,
            style: photocraft_doc::text::CharStyle { font_family: "Inter".into(), postscript_name: Some("Lato-Bold".into()), ..Default::default() },
        }];
        assert!(uses_family(&psd, &["Lato".into()]));
    }

    /// #6087: the first History snapshot is picked on its own, so a type layer deleted since the
    /// document opened still gets the font there, and one still shared with the open document is
    /// picked once.
    #[test]
    fn the_first_snapshot_is_picked_on_its_own() {
        use photocraft_doc::LayerId;
        use serde_json::json;
        let family = "Served Test Pick";
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
        let id = LayerId(s.execute("type.create", json!({"text": "Hello", "size": 24, "font": family})).unwrap()["layer"].as_u64().unwrap());
        let doc = (*s.active().unwrap().doc).clone();
        s.add_document(doc, None);
        let doc_id = s.active().unwrap().doc.id;
        let key = |first_snapshot| TypeLayerRef { doc: doc_id, layer: id, first_snapshot };
        let picked = |s: &photocraft_engine::Session| {
            let mut layers = Vec::new();
            pick(s, &[family.into()], &mut layers);
            layers.retain(|l| l.doc == doc_id);
            layers
        };
        assert_eq!(picked(&s), [key(false)], "the snapshot still is the open document");
        s.execute("layer.new.layer", json!({})).unwrap();
        assert_eq!(picked(&s), [key(false), key(true)]);
        s.execute("layer.delete", json!({"layer": id.0})).unwrap();
        assert_eq!(picked(&s), [key(true)], "deleted from the open document, still in the snapshot");
    }

    #[test]
    fn a_layer_in_a_script_redraws_when_that_scripts_fallback_arrives() {
        // Thai: other tests use other scripts (the served catalog is process-wide).
        let thai = "Served Test Thai";
        photocraft_text::served::add_fonts(&parse_manifest(&format!("{thai} | Regular | fonts/t/T.ttf | Thai\n")).0);
        let layer = |text: &str| TextLayer { text: text.into(), font_family: "Inter".into(), ..Default::default() };
        assert!(redraws_for(&layer("สวัสดี"), &[thai.into()]), "Thai text set in Inter falls back to it");
        assert!(!redraws_for(&layer("hello"), &[thai.into()]));
        assert!(redraws_for(&layer("hello"), &["Inter".into()]), "and a layer still redraws for its own family");
    }
}
