//! #14881 - BUG-LAYER-STYLE-DROP-SHADOW-ANTI-ALIASED-IGNORED.
//!
//! Measured on the issue's test document: 400 x 300 px, 72 ppi, RGB 8 bit, white Background, a
//! pixel layer with an orange (255, 128, 0) rectangle at x 100..=299, y 75..=224, Global Light
//! 120 / 30. Black Drop Shadow, Multiply 100 %, Size 10, Distance 0, Spread 0, Noise 0, Contour
//! Gaussian. The tables are the R channel of the flattened image read from a layer edge outwards
//! (255 = no shadow): top = column x 200 from y 74 up, left = row y 150 from x 99, bottom = column
//! x 200 from y 225 down, right = row y 150 from x 300. Outside the layer the shadow multiplies
//! the white Background, so R = 255 x (1 - coverage).
//!
//! On main `shadow_map` read `Shadow::anti_alias` for the Inner Shadow only (#14934), so a Drop
//! Shadow gave one map both ways: every edge read 115 161 205 231 244 250 252 253 254. The
//! reference application's Anti-aliased profile is a drop shadow whose source sits a third of a
//! pixel towards the top-left, the same offset #14934 measured for the Inner Shadow: the top and
//! left edges read 98 147 191 223 241 249 252 254 255, the bottom and right 130 177 214 236 247
//! 252 254 254 255.

use super::*;

const DOC_W: usize = 400;
const DOC_H: usize = 300;

/// The layer's alpha over the whole document: solid over x 100..=299, y 75..=224.
fn document_shape() -> Map {
    let mut m = Map::new(DOC_W, DOC_H, 0.0);
    for y in 75..=224 {
        for x in 100..=299 {
            m.v[y * DOC_W + x] = 1.0;
        }
    }
    m
}

fn curve(name: &str, pts: &[(f32, f32)]) -> Contour {
    Contour::Custom {
        name: name.into(),
        points: pts.iter().map(|(input, output)| photocraft_doc::adjust::CurvePoint { input: *input, output: *output }).collect(),
        // Every inner point of a built-in preset is a corner (the Contour picker stores it that
        // way), so the curve is its polyline.
        corners: (0..pts.len()).map(|i| i > 0 && i < pts.len() - 1).collect(),
    }
}

/// The Gaussian contour as the built-in preset carries it (`layer_style::CONTOURS`).
fn gaussian() -> Contour {
    curve(
        "Gaussian",
        &[
            (0.0, 0.0),
            (0.117, 0.023),
            (0.172, 0.055),
            (0.234, 0.117),
            (0.305, 0.234),
            (0.383, 0.414),
            (0.523, 0.688),
            (0.617, 0.812),
            (0.703, 0.891),
            (0.805, 0.953),
            (1.0, 1.0),
        ],
    )
}

/// A second built-in preset, to show the gate is "the measured contour", not "any curve".
fn cone() -> Contour {
    curve(
        "Cone",
        &[
            (0.0, 0.0),
            (0.172, 0.477),
            (0.195, 0.555),
            (0.266, 0.711),
            (0.383, 0.922),
            (0.43, 0.969),
            (0.477, 0.992),
            (0.523, 0.992),
            (0.57, 0.969),
            (0.664, 0.859),
            (0.703, 0.789),
            (0.742, 0.711),
            (0.836, 0.477),
            (0.859, 0.398),
            (0.914, 0.266),
            (0.945, 0.156),
            (0.977, 0.078),
            (1.0, 0.0),
        ],
    )
}

/// The issue's Drop Shadow: black, Multiply, 100 %, Global Light 120 / 30, Spread `spread`.
fn shadow(size: f32, distance: f32, spread: f32, contour: Contour, anti_aliased: bool) -> Shadow {
    Shadow {
        common: photocraft_doc::FxCommon::new(BlendMode::Multiply, 1.0),
        color: photocraft_color::Color::BLACK,
        angle: 120.0,
        use_global_light: true,
        distance,
        spread,
        size,
        contour,
        anti_alias: anti_aliased,
        noise: 0.0,
        knocks_out: false,
    }
}

fn map(size: f32, distance: f32, spread: f32, contour: Contour, anti_aliased: bool) -> Map {
    shadow_map(&document_shape(), &shadow(size, distance, spread, contour, anti_aliased), &GlobalLight::default(), false, (0, 0))
}

/// The R channel of the white Background under a black Multiply drop shadow of coverage `c`.
fn r(c: f32) -> f32 {
    255.0 * (1.0 - c)
}

/// The nine R values outwards from each of the issue's four edges, in the order it reads them.
fn ramps(m: &Map) -> [Vec<f32>; 4] {
    let line = |first: (i64, i64), step: (i64, i64)| (0..9).map(|k| r(m.get(first.0 + k * step.0, first.1 + k * step.1))).collect();
    [line((200, 74), (0, -1)), line((99, 150), (-1, 0)), line((200, 225), (0, 1)), line((300, 150), (1, 0))]
}

/// Assert each edge's ramp against `want` within `tol` levels.
fn assert_edges(m: &Map, want: &[&[f32]; 4], tol: f32, what: &str) {
    for ((side, got), w) in ["top", "left", "bottom", "right"].iter().zip(ramps(m)).zip(want) {
        for (d, (got, want)) in got.iter().zip(*w).enumerate() {
            assert!((got - want).abs() <= tol, "{what}, {side} edge, {d} px out: {got:.1} vs measured {want} (within {tol})");
        }
    }
}

/// A hash over every coverage value of a map: the map is bit-for-bit unchanged iff this is.
fn bit_hash(m: &Map) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in &m.v {
        h = (h ^ u64::from(v.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Done when: on the issue's test document, Gaussian with Anti-aliased gives the measured
/// top/left and bottom/right profiles within 3 levels. *Why* the two pairs of edges differ is not
/// measured; one sub-pixel source offset reproduces both, as it does for the Inner Shadow.
#[test]
fn anti_aliased_gaussian_gives_both_measured_profiles() {
    let tl: &[f32] = &[98.0, 147.0, 191.0, 223.0, 241.0, 249.0, 252.0, 254.0, 255.0];
    let br: &[f32] = &[130.0, 177.0, 214.0, 236.0, 247.0, 252.0, 254.0, 254.0, 255.0];
    assert_edges(&map(10.0, 0.0, 0.0, gaussian(), true), &[tl, tl, br, br], 3.0, "Size 10 Gaussian, Anti-aliased");
}

/// Done when: without Anti-aliased the profiles stay as today. These are the R values
/// `shadow_map` gave on main, and they sit within 3 levels of the issue's measured Anti-aliased
/// off profile, 114 163 205 231 244 251 253 254 255, on every edge.
#[test]
fn without_anti_alias_the_profiles_stay_as_today() {
    let today: [i32; 9] = [115, 161, 205, 231, 244, 250, 252, 253, 254];
    let measured: &[f32] = &[114.0, 163.0, 205.0, 231.0, 244.0, 251.0, 253.0, 254.0, 255.0];
    let m = map(10.0, 0.0, 0.0, gaussian(), false);
    for (side, got) in ["top", "left", "bottom", "right"].iter().zip(ramps(&m)) {
        for (d, got) in got.iter().enumerate() {
            assert_eq!(got.round() as i32, today[d], "Anti-aliased off moved the {side} edge, {d} px out: {got:.2}");
        }
    }
    assert_edges(&m, &[measured, measured, measured, measured], 3.0, "Size 10 Gaussian, Anti-aliased off");
    assert_eq!(bit_hash(&m), 0x5e49_20b6_9751_cc45, "the Size 10 Gaussian drop shadow map (Anti-aliased off) moved");
}

/// The offset is a third of a pixel, so the shadow reaches at most one pixel further at the top
/// and left: it stays inside the effect margin (`margin`: Size + Distance + 2) on every side.
#[test]
fn anti_aliased_stays_inside_the_effect_margin() {
    let m = map(10.0, 0.0, 0.0, gaussian(), true);
    for (x, y) in [(200i64, 75 - 12), (200, 224 + 12), (100 - 12, 150), (299 + 12, 150), (100 - 12, 75 - 12), (299 + 12, 224 + 12)] {
        assert_eq!(m.get(x, y), 0.0, "Anti-aliased drop shadow beyond the effect margin at ({x},{y})");
    }
}

/// Not measured: Anti-aliased with a contour other than Gaussian. Those keep today's map
/// bit-for-bit - the Linear default and a second built-in curve.
#[test]
fn other_contours_keep_todays_map() {
    assert_eq!(bit_hash(&map(10.0, 0.0, 0.0, Contour::Linear, true)), 0x8dc3_4139_f452_da9d, "a Linear drop shadow with Anti-aliased moved");
    assert_eq!(bit_hash(&map(10.0, 0.0, 0.0, Contour::Linear, false)), 0x8dc3_4139_f452_da9d, "a Linear drop shadow without Anti-aliased moved");
    assert_eq!(bit_hash(&map(10.0, 0.0, 0.0, cone(), true)), bit_hash(&map(10.0, 0.0, 0.0, cone(), false)), "a Cone drop shadow moved with Anti-aliased");
}

/// The flag off leaves every other Size, Distance and Spread exactly as it was: whole-map hashes
/// `shadow_map` gave on main 610c86b969 before this change.
#[test]
fn the_off_path_is_bit_for_bit_unchanged() {
    let hashes: [(f32, u64); 9] = [
        (0.5, 0xa067_0cbe_8a34_6225),
        (1.0, 0xa067_0cbe_8a34_6225),
        (1.5, 0xdc2a_66a2_b0c4_a04d),
        (2.0, 0x2b77_3190_0ec6_f501),
        (3.0, 0x202a_f725_63f0_d551),
        (6.0, 0x1ba8_550d_7781_8ee9),
        (10.0, 0x5e49_20b6_9751_cc45),
        (20.0, 0xf1a6_f4c4_f2f3_5d11),
        (250.0, 0x0842_22f2_601f_bfb1),
    ];
    for (size, want) in hashes {
        assert_eq!(bit_hash(&map(size, 0.0, 0.0, gaussian(), false)), want, "the whole Size {size} drop shadow map (Anti-aliased off) moved");
    }
    assert_eq!(bit_hash(&map(10.0, 5.0, 0.0, gaussian(), false)), 0xda80_8e81_2a08_7185, "the Size 10 Distance 5 drop shadow map moved");
    assert_eq!(bit_hash(&map(10.0, 0.0, 0.5, gaussian(), false)), 0x5419_4d5e_753c_77a1, "the Size 10 Spread 50 % drop shadow map moved");
}

/// The Inner Shadow's gate (`aa_shift`) applies unchanged: a Size whose blur is the identity
/// (Size 1, Size 2 at Spread 50 %) would turn the offset into the whole map, and a Spread that
/// dilates makes the CPU recompute its distance on the offset map while the GPU shifts a
/// whole-pixel field of the unshifted shape. None of them was measured, so they keep today's map.
#[test]
fn degenerate_and_dilating_neighbours_keep_todays_map() {
    for (size, spread, what) in [(1.0, 0.0, "Size 1"), (2.0, 0.5, "Size 2 Spread 50 %"), (10.0, 0.5, "Size 10 Spread 50 %"), (10.0, 0.05, "Size 10 Spread 5 %")]
    {
        assert_eq!(
            bit_hash(&map(size, 0.0, spread, gaussian(), true)),
            bit_hash(&map(size, 0.0, spread, gaussian(), false)),
            "the {what} drop shadow map moved with Anti-aliased"
        );
    }
}

/// Guard: the rule is in the inputs, not keyed to the measured Size 10. Every Size whose blur is
/// wider than the identity takes the offset at Spread 0, and so does a Distance other than 0.
#[test]
fn the_offset_follows_the_inputs_not_the_measured_size() {
    for size in [1.5, 2.0, 3.0, 6.0, 20.0, 250.0] {
        assert_ne!(
            bit_hash(&map(size, 0.0, 0.0, gaussian(), true)),
            bit_hash(&map(size, 0.0, 0.0, gaussian(), false)),
            "Anti-aliased did nothing at Size {size}, Spread 0"
        );
    }
    assert_ne!(
        bit_hash(&map(10.0, 5.0, 0.0, gaussian(), true)),
        bit_hash(&map(10.0, 5.0, 0.0, gaussian(), false)),
        "Anti-aliased did nothing at Size 10, Distance 5"
    );
}
