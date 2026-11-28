//! #17311: own synthetic 30 px strokes, checked against the published measured centres.

use crate::dynamics::DabGenerator;
use crate::{BrushSettings, Control, Dab, Dynamic, Stroke, StrokePoint, dabs};

fn brush() -> BrushSettings {
    let mut brush = BrushSettings { size: 30.0, spacing: 1.5, seed: 17311, ..BrushSettings::default() };
    brush.shape_dynamics.enabled = true;
    brush.shape_dynamics.size = Dynamic { control: Control::Fade, fade_steps: 10, ..Dynamic::default() };
    brush
}

fn line(brush: BrushSettings) -> Vec<Dab> {
    dabs(&Stroke { brush, points: vec![StrokePoint::new(100.0, 100.0, 1.0), StrokePoint::new(700.0, 100.0, 1.0)] })
}

#[test]
fn size_fade_spacing_tracks_the_first_ten_measured_dab_centres() {
    let steps = line(brush());
    let measured = [100.0, 144.0, 185.0, 221.0, 252.0, 280.0, 302.0, 320.0, 334.0, 342.0];
    // Termination after these dabs is #17312; this independent fix tests the spacing only.
    assert!(steps.len() >= measured.len());
    for (dab, expected) in steps.iter().take(measured.len()).zip(measured) {
        assert!((dab.center.x - expected).abs() <= 1.0 + 1e-4, "{} vs {expected}", dab.center.x);
    }
}

#[test]
fn a_positive_minimum_keeps_spacing_with_the_faded_size_across_input_chunks() {
    let mut brush = brush();
    brush.shape_dynamics.size.minimum = 0.3;
    let reference = line(brush.clone());
    assert_eq!(reference.len(), 32);
    for pair in reference.windows(2) {
        let pitch = f64::from(pair[0].radius * 2.0 * brush.spacing);
        assert!((pair[1].center.x - pair[0].center.x - pitch).abs() < 1e-4);
    }
    assert!(reference.last().unwrap().center.x > 680.0);
    assert!((reference.last().unwrap().radius * 2.0 - 9.0).abs() < 1e-4);
    let mut generator = DabGenerator::new(&brush, 1.0);
    let (mut chunked, mut dual) = (Vec::new(), Vec::new());
    for x in (100..=700).step_by(50) {
        generator.push(&[StrokePoint::new(f64::from(x), 100.0, 1.0)], &mut chunked, &mut dual);
    }
    generator.finish(&mut chunked, &mut dual);
    assert_eq!(chunked.len(), reference.len());
    for (a, b) in chunked.iter().zip(reference) {
        assert!((a.center.x - b.center.x).abs() < 1e-4);
        assert_eq!(a.radius, b.radius);
    }
}

#[test]
fn opacity_flow_angle_and_roundness_fades_do_not_change_spacing() {
    let mut plain = brush();
    plain.shape_dynamics.size.control = Control::Off;
    let positions = |brush| line(brush).into_iter().map(|dab| dab.center).collect::<Vec<_>>();
    let expected = positions(plain.clone());
    let fade = Dynamic { control: Control::Fade, fade_steps: 3, ..Dynamic::default() };
    for control in 0..4 {
        let mut variant = plain.clone();
        match control {
            0 => {
                variant.transfer.enabled = true;
                variant.transfer.opacity = fade;
            }
            1 => {
                variant.transfer.enabled = true;
                variant.transfer.flow = fade;
            }
            2 => variant.shape_dynamics.angle = fade,
            _ => variant.shape_dynamics.roundness = fade,
        }
        assert_eq!(positions(variant), expected);
    }
}
