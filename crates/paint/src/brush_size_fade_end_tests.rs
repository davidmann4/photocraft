//! #17312: own synthetic strokes must not leave the old radius-floor dots after Size Fade.

use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

use crate::{BrushSettings, Control, Dynamic, Stroke, StrokePoint, dabs, render_stroke};

fn brush(steps: u32, minimum: f32) -> BrushSettings {
    let mut brush = BrushSettings { size: 30.0, spacing: 1.5, seed: 17312, ..BrushSettings::default() };
    brush.shape_dynamics.enabled = true;
    brush.shape_dynamics.size = Dynamic { control: Control::Fade, fade_steps: steps, minimum, ..Dynamic::default() };
    brush
}

fn points() -> Vec<StrokePoint> {
    vec![StrokePoint::new(100.0, 100.0, 1.0), StrokePoint::new(700.0, 100.0, 1.0)]
}

#[test]
fn zero_minimum_size_fade_ends_after_the_last_nonzero_dab() {
    for count in [5, 10] {
        let brush = brush(count, 0.0);
        let steps = dabs(&Stroke { brush: brush.clone(), points: points() });
        assert_eq!(steps.len(), count as usize);
        for (index, dab) in steps.iter().enumerate() {
            let diameter = 30.0 * (1.0 - index as f32 / count as f32);
            assert!((dab.radius * 2.0 - diameter).abs() < 1e-4);
        }
        let mut surface = Surface::new(PixelFormat::RGBA32F);
        render_stroke(&mut surface, &brush, &points(), None, false, 1.0);
        let last = steps.last().unwrap();
        assert!(surface.rgba(last.center.x.round() as i32, 100)[3] > 0.0);
        // This independent fix retains the baseline pitch; #17311 changes its spacing.
        // Pixels beyond the last nonzero dab must stay clear, wherever that dab is placed.
        let tail_start = (last.center.x + f64::from(last.radius) + 4.0).ceil() as i32;
        for x in tail_start..=700 {
            for y in 75..=125 {
                assert_eq!(surface.rgba(x, y)[3], 0.0, "unexpected tail at {x},{y}, Fade{count}");
            }
        }
    }
}

#[test]
fn a_positive_minimum_keeps_painting_past_the_fade() {
    let steps = dabs(&Stroke { brush: brush(10, 0.3), points: points() });
    assert!(steps.len() > 10);
    assert!(steps.last().unwrap().center.x > 680.0);
    assert!(steps.iter().skip(10).all(|dab| (dab.radius - 4.5).abs() < 1e-4));
}

#[test]
fn zero_pressure_and_roundness_fade_keep_their_existing_floors() {
    let pressure = BrushSettings { size: 30.0, pressure_size: true, ..BrushSettings::default() };
    let steps = dabs(&Stroke { brush: pressure, points: vec![StrokePoint::new(0.0, 0.0, 0.0), StrokePoint::new(10.0, 0.0, 0.0)] });
    assert!(!steps.is_empty());
    assert!(steps.iter().all(|dab| dab.radius == 0.5));
    let mut roundness = brush(10, 0.0);
    roundness.shape_dynamics.roundness = roundness.shape_dynamics.size;
    roundness.shape_dynamics.size.control = Control::Off;
    let steps = dabs(&Stroke { brush: roundness, points: points() });
    assert!(steps.len() > 10);
    assert!(steps.iter().skip(10).all(|dab| dab.radius == 15.0 && dab.roundness == 0.01));
}
