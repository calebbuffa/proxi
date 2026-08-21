//! Feature-gated integration tests for ecosystem coordinate adapters and serde.

#[cfg(any(feature = "geo", feature = "nalgebra", feature = "glam"))]
use proxi::TransformerBuilder;

#[cfg(feature = "geo")]
#[test]
fn geo_types_transform() {
    let context = proxi::Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .build()
        .expect("build");

    // geo_types::Coord and ::Point<f64> are both `Coord`.
    let p: geo::Coord = geo::coord! { x: 0.0, y: 0.0 };
    let out = t
        .transform_coord(p, proxi::Direction::Forward, proxi::AngularUnits::Auto)
        .expect("geo::Coord transform");
    assert!((out.x - 0.0).abs() < 1e-9 && (out.y - 0.0).abs() < 1e-9);

    let pt: geo::Point = geo::point! { x: 0.0, y: 1.0 };
    let out_pt = t
        .transform_coord(pt, proxi::Direction::Forward, proxi::AngularUnits::Auto)
        .expect("geo::Point transform");
    assert!(out_pt.x().is_finite() && out_pt.y().is_finite());
}

#[cfg(feature = "nalgebra")]
#[test]
fn nalgebra_types_transform() {
    let context = proxi::Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .build()
        .expect("build");

    let v2 = nalgebra::Vector2::new(0.0, 0.0);
    let out = t
        .transform_coord(v2, proxi::Direction::Forward, proxi::AngularUnits::Auto)
        .expect("nalgebra Vector2");
    assert!((out.x - 0.0).abs() < 1e-9 && (out.y - 0.0).abs() < 1e-9);

    let p3 = nalgebra::Point3::new(0.0, 0.0, 10.0);
    let out_p = t
        .transform_coord(p3, proxi::Direction::Forward, proxi::AngularUnits::Auto)
        .expect("nalgebra Point3");
    assert!(out_p.z.is_finite());
}

#[cfg(feature = "glam")]
#[test]
fn glam_types_transform() {
    let context = proxi::Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .build()
        .expect("build");

    let v2 = glam::DVec2::new(0.0, 0.0);
    let out = t
        .transform_coord(v2, proxi::Direction::Forward, proxi::AngularUnits::Auto)
        .expect("glam DVec2");
    assert!((out.x - 0.0).abs() < 1e-9 && (out.y - 0.0).abs() < 1e-9);

    let v3 = glam::DVec3::new(0.0, 0.0, 5.0);
    let out3 = t
        .transform_coord(v3, proxi::Direction::Forward, proxi::AngularUnits::Auto)
        .expect("glam DVec3");
    assert!(out3.z.is_finite());
}

#[cfg(feature = "serde")]
#[test]
fn coord_types_serde_roundtrip() {
    let c2 = proxi::Coord2::new(1.5, -2.5);
    let json = serde_json::to_string(&c2).expect("serialize Coord2");
    let back: proxi::Coord2 = serde_json::from_str(&json).expect("deserialize Coord2");
    assert_eq!(c2, back);

    let c4 = proxi::Coord4::new(1.5, -2.5, 3.5, 2020.0);
    let json4 = serde_json::to_string(&c4).expect("serialize Coord4");
    let back4: proxi::Coord4 = serde_json::from_str(&json4).expect("deserialize Coord4");
    assert_eq!(c4, back4);
}
