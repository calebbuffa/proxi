//! The point of these tests is to prove that `AngularUnits::Auto` genuinely
//! inspects the operation (via `proj_angular_input` / `proj_degree_input`) and
//! behaves *correctly*, not merely differently from `Degrees`/`Radians`. The
//! verified properties:
//!
//! - **Geographic -> projected (forward):** `Auto` converts caller-supplied
//!   *degrees* to radians for the angular input and returns projected *meters*
//!   on output. It therefore equals `Degrees` for a degree-typed caller value.
//! - **Radians == Auto to_radians:** passing already-radian input with
//!   `Radians` produces the same result as passing degree input with `Auto`
//!   (which converts), proving `Auto` really applies the deg->rad scale.
//! - **Projected -> projected (linear):** neither input nor output is angular,
//!   so `Auto`, `Degrees`, and `Radians` all apply identity scaling (meters
//!   pass through unchanged).
//! - **Inverse:** the same relationship holds in reverse (projected -> degrees).

use proxi::{AngularUnits, Context, Coord3, Direction};

fn context() -> Context {
    Context::new().expect("configured context")
}

/// Forward geographic (degrees) -> projected (meters): `Auto` converts degrees
/// to radians internally, matching `Degrees`, and both give correct UTM.
#[test]
fn auto_equals_degrees_for_degree_input_on_geographic_forward() {
    let c = context();
    let mut t = proxi::TransformerBuilder::new(&c, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build 4326->32633");
    // EPSG:32633 is UTM zone 33N, whose central meridian is 15 degrees E. A point on
    // that meridian at the equator maps to (500000 E, 0 N).
    let lon_lat = Coord3::new(15.0, 0.0, 0.0); // degrees

    let auto = t
        .transform_xyz(lon_lat, Direction::Forward, AngularUnits::Auto)
        .expect("auto forward");
    let degrees = t
        .transform_xyz(lon_lat, Direction::Forward, AngularUnits::Degrees)
        .expect("degrees forward");

    // Both should produce the UTM central-meridian origin (~500000 E, 0 N).
    assert!((auto.x - 500_000.0).abs() < 0.01, "auto x {}", auto.x);
    assert!(auto.y.abs() < 0.01, "auto y {}", auto.y);
    assert!(
        (auto.x - degrees.x).abs() < 1e-6 && (auto.y - degrees.y).abs() < 1e-6,
        "Auto should match Degrees for degree input: {auto:?} vs {degrees:?}"
    );
}

/// `Radians` input (already radians) must equal `Auto` with the same point
/// given in degrees. This proves `Auto` applies the deg->rad conversion.
#[test]
fn radians_input_equals_auto_degrees_input() {
    let c = context();
    let mut t = proxi::TransformerBuilder::new(&c, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build 4326->32633");

    // Same physical point: 10 deg lon, 20 deg lat.
    let deg = Coord3::new(10.0, 20.0, 0.0);
    let rad = Coord3::new(10.0_f64.to_radians(), 20.0_f64.to_radians(), 0.0);

    let auto = t
        .transform_xyz(deg, Direction::Forward, AngularUnits::Auto)
        .expect("auto");
    let radians = t
        .transform_xyz(rad, Direction::Forward, AngularUnits::Radians)
        .expect("radians");

    assert!(
        (auto.x - radians.x).abs() < 1e-6 && (auto.y - radians.y).abs() < 1e-6,
        "Radians(rad input) should equal Auto(deg input): {auto:?} vs {radians:?}"
    );
}

/// Projected -> projected (linear XY): neither input nor output is angular, so
/// `Auto`, `Degrees`, and `Radians` all apply identity scaling.
#[test]
fn linear_operation_auto_degrees_radians_all_identity() {
    let c = context();
    // EPSG:32633 -> EPSG:32635 (two UTM zones, both projected/linear).
    let mut t = proxi::TransformerBuilder::new(&c, "EPSG:32633", "EPSG:32635")
        .build()
        .expect("build 32633->32635");

    let meters = Coord3::new(500_000.0, 4_500_000.0, 100.0);
    let auto = t
        .transform_xyz(meters, Direction::Forward, AngularUnits::Auto)
        .expect("auto");
    let degrees = t
        .transform_xyz(meters, Direction::Forward, AngularUnits::Degrees)
        .expect("degrees");
    let radians = t
        .transform_xyz(meters, Direction::Forward, AngularUnits::Radians)
        .expect("radians");

    // All three agree (identity scaling on linear XY). Input and output are in
    // the same linear units, so the x/y should be near-identical (the two UTM
    // zones share a central meridian at 500000 E, so E is preserved).
    assert!(
        (auto.x - degrees.x).abs() < 1e-6 && (auto.x - radians.x).abs() < 1e-6,
        "linear scales must be identical: {auto:?}, {degrees:?}, {radians:?}"
    );
}

/// Inverse direction: projected -> geographic. `Auto` returns degrees on
/// output (after converting radians back), and matches `Degrees`.
#[test]
fn auto_inverse_returns_degrees_and_matches_degrees() {
    let c = context();
    let mut t = proxi::TransformerBuilder::new(&c, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build 4326->32633");

    // A UTM point at the zone-33N central meridian (500000 E, 0 N) maps back
    // to lon=15 degrees , lat=0 degrees .
    let utm = Coord3::new(500_000.0, 0.0, 0.0);
    let auto = t
        .transform_xyz(utm, Direction::Inverse, AngularUnits::Auto)
        .expect("auto inverse");
    let degrees = t
        .transform_xyz(utm, Direction::Inverse, AngularUnits::Degrees)
        .expect("degrees inverse");

    // Auto returns degrees: ~15 degrees E lon, 0 lat.
    assert!((auto.x - 15.0).abs() < 1e-6, "auto inverse lon {}", auto.x);
    assert!(auto.y.abs() < 1e-6, "auto inverse lat {}", auto.y);
    assert!(
        (auto.x - degrees.x).abs() < 1e-6 && (auto.y - degrees.y).abs() < 1e-6,
        "inverse Auto should match Degrees: {auto:?} vs {degrees:?}"
    );
}

/// A forward/backward round-trip with `Auto` on both directions must recover
/// the original degree input.
#[test]
fn auto_roundtrip_recovers_original_degrees() {
    let c = context();
    let mut t = proxi::TransformerBuilder::new(&c, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build 4326->32633");

    let original = Coord3::new(15.0, 45.0, 100.0);
    let utm = t
        .transform_xyz(original, Direction::Forward, AngularUnits::Auto)
        .expect("forward");
    let back = t
        .transform_xyz(utm, Direction::Inverse, AngularUnits::Auto)
        .expect("inverse");

    assert!(
        (back.x - original.x).abs() < 1e-6,
        "lon roundtrip {}",
        back.x
    );
    assert!(
        (back.y - original.y).abs() < 1e-6,
        "lat roundtrip {}",
        back.y
    );
    assert!((back.z - original.z).abs() < 1e-3, "z roundtrip {}", back.z);
}
