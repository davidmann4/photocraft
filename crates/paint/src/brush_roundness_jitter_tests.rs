//! #17320: the jitter-only roundness range, with Minimum Roundness and unmeasured controls preserved.

use crate::brush::{BrushSettings, Control, Dynamic};
use crate::dynamics::{DabBuilder, StepInput, dynamic_value};
use crate::rng::{rand01, stream};
use crate::{Dab, StrokePoint};

fn brush() -> BrushSettings {
    let mut brush = BrushSettings { size: 40.0, seed: 0x17320, ..Default::default() };
    brush.shape_dynamics.enabled = true;
    brush.shape_dynamics.roundness = Dynamic::jitter(1.0);
    brush
}

/// Drive the real primary-dab builder without involving stroke spacing.
fn dabs(brush: &BrushSettings) -> Vec<Dab> {
    let mut builder = DabBuilder::new(brush);
    let mut out = Vec::new();
    for step in 0..64_u16 {
        builder.build(
            &StepInput { point: StrokePoint::new(f64::from(step) * 80.0, 40.0, 1.0), direction: 0.0, initial_direction: 0.0, step: u64::from(step) },
            &mut out,
        );
    }
    out
}

#[test]
fn full_roundness_jitter_keeps_a_quarter_of_the_base_tip() {
    let brush = brush();
    let round = dabs(&brush);
    assert!(round.len() >= 50);
    assert!(round.iter().all(|d| d.roundness >= 0.25 && d.roundness <= 1.0));
    assert!(round.iter().any(|d| d.roundness < 0.4), "jitter still makes visibly flat dabs");
    assert!(round.iter().any(|d| d.roundness > 0.9), "the range still reaches nearly the full tip");
    assert!(round.iter().all(|d| d.radius * 2.0 * d.roundness >= brush.size * 0.25));

    // Keep the bound relative to the base so jitter never makes a flat tip rounder.
    let flat = BrushSettings { roundness: 0.2, ..brush };
    let flat_dabs = dabs(&flat);
    assert!(flat_dabs.iter().all(|d| d.roundness >= flat.roundness * 0.25 && d.roundness <= flat.roundness));
    assert!(flat_dabs.iter().any(|d| d.roundness < flat.roundness * 0.5));
}

#[test]
fn minimum_roundness_above_a_quarter_remains_the_floor() {
    let mut brush = brush();
    brush.shape_dynamics.roundness.minimum = 0.5;
    let varied = dabs(&brush);
    assert!(varied.len() >= 50);
    assert!(varied.iter().all(|d| d.roundness >= 0.5 && d.roundness <= 1.0));
    assert!(varied.iter().any(|d| d.roundness < 0.55), "the 40 px tip still flattens to about 20 px");
    assert!(varied.iter().any(|d| d.roundness > 0.9));

    brush.shape_dynamics.roundness.jitter = 0.0;
    assert!(dabs(&brush).iter().all(|d| d.roundness == brush.roundness), "no jitter leaves the base unchanged");
}

#[test]
fn roundness_fade_keeps_its_existing_jitter_and_terminal_range() {
    for jitter in [0.0, 1.0] {
        let mut brush = brush();
        brush.shape_dynamics.roundness = Dynamic { control: Control::Fade, fade_steps: 16, jitter, ..Default::default() };
        let faded = dabs(&brush);
        assert!(faded.len() >= 50);
        // This is the retained Fade contract, not a newly fitted reference distribution.
        for (step, dab) in (0..64_u16).zip(&faded) {
            let input = StepInput { point: StrokePoint::new(f64::from(step) * 80.0, 40.0, 1.0), direction: 0.0, initial_direction: 0.0, step: u64::from(step) };
            let control = dynamic_value(&brush.shape_dynamics.roundness, &input);
            let old = (brush.roundness * control * (1.0 - jitter * rand01(brush.seed, dab.index, stream::ROUNDNESS))).clamp(0.01, 1.0);
            assert!((dab.roundness - old).abs() < 1e-6, "Fade step {step}, jitter {jitter}");
        }
        assert!(faded.iter().any(|d| d.roundness < 0.25), "a jitter bound must not clamp Roundness Fade");
    }
}

#[test]
fn roundness_jitter_does_not_change_other_random_streams_or_dab_indices() {
    let mut varied = brush();
    varied.shape_dynamics.size.jitter = 0.4;
    varied.shape_dynamics.angle.jitter = 0.5;
    varied.shape_dynamics.flip_x_jitter = true;
    varied.shape_dynamics.flip_y_jitter = true;
    varied.scattering.enabled = true;
    varied.scattering.scatter.jitter = 0.6;
    varied.scattering.count = 3;
    varied.scattering.count_jitter = Dynamic::jitter(0.5);
    varied.transfer.enabled = true;
    varied.transfer.flow.jitter = 0.3;
    let mut plain = varied.clone();
    plain.shape_dynamics.roundness.jitter = 0.0;
    let actual = dabs(&varied);
    let control = dabs(&plain);
    assert_eq!(actual.len(), control.len());
    for (mut dab, same) in actual.into_iter().zip(control) {
        dab.roundness = same.roundness;
        assert_eq!(dab, same, "only roundness may change");
    }
}
