//! Export As (#17199): Document Profile describes untagged pixels in the session working space.

use super::replay;
use crate::{PhotocraftApp, Services};
use craft::cms::{Builtin, Intent};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::json;
use std::{cell::RefCell, rc::Rc, sync::Arc};

type Writes = Rc<RefCell<Vec<Vec<u8>>>>;

fn app(mode: ColorMode, tagged: Option<Builtin>) -> (Box<PhotocraftApp>, Writes) {
    let writes = Writes::default();
    let captured = writes.clone();
    let services = Services {
        export: Some(Box::new(|doc, path, settings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|result| (result.bytes, result.warnings)).map_err(|error| error.to_string())
        })),
        write: Some(Box::new(move |_, bytes| {
            captured.borrow_mut().push(bytes.to_vec());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session
        .execute(
            "edit.colorSettings",
            json!({"workingRgb": Builtin::ProPhotoCompat.id(), "workingGray": Builtin::GrayGamma22.id(), "policyRgb": "off", "policyGray": "off"}),
        )
        .unwrap();
    let (left, right) = match mode {
        ColorMode::Grayscale => ([0.25, 0.25, 0.25, 1.0], [0.65, 0.65, 0.65, 1.0]),
        _ => ([200.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0], [30.0 / 255.0, 60.0 / 255.0, 200.0 / 255.0, 1.0]),
    };
    let mut doc = Document::with_background("working-profile", Size::new(8, 4), mode, SampleType::U8, Color::rgba(left[0], left[1], left[2], left[3]));
    let surface = doc.layers[0].surface_mut().unwrap();
    surface.fill_rect(photocraft_geom::Rect::from_xywh(4, 0, 4, 4), &photocraft_raster::from_rgba(&surface.format(), right));
    doc.icc_profile = tagged.map(|profile| profile.profile().to_bytes());
    // Add the exact fixture without an import-policy conversion or mismatch prompt.
    app.session.add_document(doc, None);
    (app, writes)
}

fn pixels(doc: &Document) -> Vec<[f32; 4]> {
    let surface = doc.layers[0].surface().unwrap();
    (0..doc.size.height).flat_map(|y| (0..doc.size.width).map(move |x| surface.rgba(x as i32, y as i32))).collect()
}

fn replay_png(app: &mut PhotocraftApp, writes: &Writes, profile: &str, embed: bool) -> Document {
    replay(app, &json!({"path": "out.png", "format": "png", "profile": profile, "embedProfile": embed, "transparency": true, "metadata": "none"})).unwrap();
    let bytes = writes.borrow_mut().pop().expect("replay writes the encoded PNG");
    photocraft_io::import("out.png", &bytes).unwrap().document
}

#[test]
fn document_profile_png_embeds_working_or_tagged_rgb_and_gray_without_changing_pixels() {
    for (mode, working, own) in [(ColorMode::Rgb, Builtin::ProPhotoCompat, Builtin::DisplayP3), (ColorMode::Grayscale, Builtin::GrayGamma22, Builtin::SGray)] {
        for tagged in [None, Some(own)] {
            let (mut app, writes) = app(mode, tagged);
            let source = app.session.active().unwrap().doc.clone();
            let before = (*source).clone();
            assert_eq!(source.icc_profile, tagged.map(|profile| profile.profile().to_bytes()), "the fixture keeps its original tag state");
            let samples = pixels(&source);
            let expected_icc = tagged.unwrap_or(working).profile().to_bytes();
            for embed in [true, false] {
                let output = replay_png(&mut app, &writes, "document", embed);
                assert_eq!(output.size, source.size);
                assert_eq!(output.mode, mode);
                assert_eq!(
                    output.icc_profile.as_ref().map(|icc| icc.as_slice()),
                    embed.then_some(expected_icc.as_slice()),
                    "{mode:?} {tagged:?} embed={embed}"
                );
                assert_eq!(pixels(&output), samples, "Document Profile must tag without converting the samples");
                assert!(Arc::ptr_eq(&source, &app.session.active().unwrap().doc), "export must retain the source document");
                assert_eq!(pixels(&source), samples, "the source samples stay unchanged");
                assert_eq!(*source, before, "export must not change the source ICC or pixels");
            }
        }
    }
}

#[test]
fn explicit_srgb_png_converts_from_working_rgb_and_gray_and_keeps_embedding_optional() {
    for (mode, working) in [(ColorMode::Rgb, Builtin::ProPhotoCompat), (ColorMode::Grayscale, Builtin::GrayGamma22)] {
        let (mut app, writes) = app(mode, None);
        let source = app.session.active().unwrap().doc.clone();
        let before = (*source).clone();
        assert!(source.icc_profile.is_none(), "the conversion source is actually untagged");
        let original_pixels = pixels(&source);
        let mut expected = before.clone();
        expected.icc_profile = Some(working.profile().to_bytes());
        photocraft_engine::color_cmds::convert_document(&mut expected, Builtin::Srgb.profile(), Intent::Perceptual, true).unwrap();
        let samples = pixels(&expected);
        assert_ne!(samples, original_pixels, "the fixture must distinguish conversion from working-profile tagging");
        let srgb = Builtin::Srgb.profile().to_bytes();
        for embed in [true, false] {
            let output = replay_png(&mut app, &writes, "srgb", embed);
            assert_eq!(output.size, source.size);
            assert_eq!(output.mode, ColorMode::Rgb);
            assert_eq!(output.icc_profile.as_ref().map(|icc| icc.as_slice()), embed.then_some(srgb.as_slice()), "{mode:?} embed={embed}");
            assert_eq!(pixels(&output), samples, "explicit sRGB must keep its pixel conversion, not just change the tag");
            assert!(Arc::ptr_eq(&source, &app.session.active().unwrap().doc));
            assert_eq!(pixels(&source), original_pixels, "the source samples stay unchanged");
            assert_eq!(*source, before, "export must not change the source ICC or pixels");
        }
    }
}
