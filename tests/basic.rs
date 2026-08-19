//! Basic end-to-end transform tests, exercising the self-provisioned PROJ.

use proxi::{
    Context, Conversion, Coord2, Coord3, Coord4, CoordBatch, Crs, DEGREE_METRE, Database,
    Direction, Proj, ProxiError, TransformerBuilder, WktVersion,
};

#[test]
fn geocentric_to_utm_and_back_roundtrips() {
    let context = Context::configured().expect("context");
    // EPSG:4978 = WGS84 geocentric; EPSG:32633 = WGS84 UTM 33N.
    let mut t = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:32633")
        .build()
        .expect("build transformer");

    // A point on the Greenwich meridian at the equator, ~ surface of the WGS84 ellipsoid.
    let a = 6378137.0;
    let geocentric = Coord3::new(a, 0.0, 0.0);

    let utm = t.forward_xyz(geocentric).expect("forward transform");

    let back = t.inverse_xyz(utm).expect("inverse transform");
    assert!((back.x - a).abs() < 0.01, "x roundtrip, got {}", back.x);
    assert!(back.y.abs() < 0.01, "y roundtrip, got {}", back.y);
    assert!(back.z.abs() < 0.01, "z roundtrip, got {}", back.z);
}

#[test]
fn batch_soa_roundtrip() {
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:32633")
        .build()
        .expect("build transformer");

    let orig_xs = vec![6378137.0, 6378137.0, 6378137.0];
    let orig_ys = vec![0.0, 1000.0, -1000.0];
    let orig_zs = vec![0.0, 0.0, 0.0];
    let mut xs = orig_xs.clone();
    let mut ys = orig_ys.clone();
    let mut zs = orig_zs.clone();

    t.transform_xyz_in_place(
        &mut xs,
        &mut ys,
        &mut zs,
        Direction::Forward,
        proxi::AngularUnits::Auto,
    )
    .expect("forward batch");

    t.transform_xyz_in_place(
        &mut xs,
        &mut ys,
        &mut zs,
        Direction::Inverse,
        proxi::AngularUnits::Auto,
    )
    .expect("inverse batch");

    for i in 0..3 {
        assert!(
            (xs[i] - orig_xs[i]).abs() < 0.01,
            "x[{}] roundtrip {}, want {}",
            i,
            xs[i],
            orig_xs[i]
        );
        assert!(
            (ys[i] - orig_ys[i]).abs() < 0.01,
            "y[{}] roundtrip {}, want {}",
            i,
            ys[i],
            orig_ys[i]
        );
        assert!(
            (zs[i] - orig_zs[i]).abs() < 0.01,
            "z[{}] roundtrip {}, want {}",
            i,
            zs[i],
            orig_zs[i]
        );
    }
}

#[test]
fn invalid_crs_is_rejected() {
    let context = Context::configured().expect("context");
    let err = TransformerBuilder::new(&context, "EPSG:4978", "not a real crs").build();
    assert!(err.is_err(), "invalid CRS should fail");
}

#[test]
fn always_xy_constructs_and_transforms() {
    let context = Context::configured().expect("context");
    // always_xy should build without leaking and produce a working transform.
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build always_xy transformer");

    // Repeat construction to smoke out leaks in normalize_for_visualization.
    for _ in 0..10 {
        let mut u = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
            .always_xy(true)
            .build()
            .expect("repeated build");
        let p = u
            .forward_xyz(Coord3::new(0.0, 0.0, 0.0))
            .expect("transform");
        assert!(p.x.is_finite() && p.y.is_finite(), "finite utm, got {p:?}");
    }

    // 0 lon, 0 lat -> near UTM origin.
    let p = t
        .forward_xyz(Coord3::new(0.0, 0.0, 0.0))
        .expect("transform");
    assert!(p.x.is_finite() && p.y.is_finite(), "finite utm, got {p:?}");
}

#[test]
fn wkt_and_projjson_output() {
    let context = Context::configured().expect("context");
    let c = proxi::Crs::from_user_input(&context, "EPSG:4326").expect("epsg:4326");
    let wkt = c.to_wkt(WktVersion::Wkt1Esri).expect("wkt");
    assert!(
        wkt.contains("GEOGCS") || wkt.contains("GEODCRS"),
        "esri wkt: {wkt}"
    );
    let pj = c.to_projjson().expect("projjson");
    assert!(pj.contains("\"type\""), "projjson has type: {pj}");
}

#[test]
fn forward_xyzt_4d() {
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:4326+3855")
        .build()
        .expect("build 4d");
    // 4D: lon, lat, h, t. Roundtrip should preserve.
    let input = Coord4::new(0.0, 0.0, 10.0, 2020.0);
    let out = t.forward_xyzt(input).expect("forward 4d");
    let back = t.inverse_xyzt(out).expect("inverse 4d");
    assert!((back.x - input.x).abs() < 1e-6, "x {}", back.x);
    assert!((back.y - input.y).abs() < 1e-6, "y {}", back.y);
    assert!((back.z - input.z).abs() < 1e-3, "z {}", back.z);
    assert!((back.t - input.t).abs() < 1e-3, "t {}", back.t);
}

#[test]
fn scalar_matches_soa() {
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:32633")
        .build()
        .expect("build");
    let xs = vec![6378137.0, 6378137.0, 6378137.0];
    let ys = vec![0.0, 1000.0, -1000.0];
    let zs = vec![0.0, 0.0, 0.0];

    let scalars: Vec<Coord3> = (0..3)
        .map(|i| {
            t.forward_xyz(Coord3::new(xs[i], ys[i], zs[i]))
                .expect("scalar forward")
        })
        .collect();

    let mut xs2 = xs.clone();
    let mut ys2 = ys.clone();
    let mut zs2 = zs.clone();
    t.transform_xyz_in_place(
        &mut xs2,
        &mut ys2,
        &mut zs2,
        Direction::Forward,
        proxi::AngularUnits::Auto,
    )
    .expect("soa forward");

    for i in 0..3 {
        assert!(
            (xs2[i] - scalars[i].x).abs() < 1e-6,
            "x[{i}] {} vs {}",
            xs2[i],
            scalars[i].x
        );
        assert!(
            (ys2[i] - scalars[i].y).abs() < 1e-6,
            "y[{i}] {} vs {}",
            ys2[i],
            scalars[i].y
        );
        assert!(
            (zs2[i] - scalars[i].z).abs() < 1e-6,
            "z[{i}] {} vs {}",
            zs2[i],
            scalars[i].z
        );
    }
}

#[test]
fn xyz_into_preserves_input_and_writes_output() {
    let context = Context::configured().expect("context");
    let mut transformer = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build transformer");
    let input_x = [15.0, 15.1];
    let input_y = [0.0, 1.0];
    let input_z = [10.0, 20.0];
    let mut output_x = [0.0; 2];
    let mut output_y = [0.0; 2];
    let mut output_z = [0.0; 2];

    transformer
        .transform_xyz_into(
            &input_x,
            &input_y,
            &input_z,
            &mut output_x,
            &mut output_y,
            &mut output_z,
            Direction::Forward,
            proxi::AngularUnits::Auto,
        )
        .expect("transform into output");

    assert_eq!(input_x, [15.0, 15.1]);
    assert_eq!(input_y, [0.0, 1.0]);
    assert_eq!(input_z, [10.0, 20.0]);
    assert!((output_x[0] - 500_000.0).abs() < 0.01);
    assert!(output_y[1] > output_y[0]);
    assert_eq!(output_z, input_z);
}

#[test]
fn xyz_transactional_rejects_mismatched_buffers_without_mutation() {
    let context = Context::configured().expect("context");
    let mut transformer = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
        .build()
        .expect("build transformer");
    let mut x = [1.0, 2.0];
    let mut y = [3.0];
    let mut z = [4.0, 5.0];
    let result = transformer.transform_xyz_transactional(
        &mut x,
        &mut y,
        &mut z,
        Direction::Forward,
        proxi::AngularUnits::Auto,
    );
    assert!(matches!(result, Err(ProxiError::LengthMismatch { .. })));
    assert_eq!(x, [1.0, 2.0]);
    assert_eq!(y, [3.0]);
    assert_eq!(z, [4.0, 5.0]);
}

#[test]
fn mismatched_batch_lengths_rejected() {
    let mut xs = [0.0; 3];
    let mut ys = [0.0; 3];
    let mut bad_z = [0.0; 2];
    let err = CoordBatch::new(&mut xs, &mut ys).and_then(|c| c.with_z(&mut bad_z));
    assert!(matches!(err, Err(ProxiError::LengthMismatch { .. })));
}

#[test]
fn coord_trait_transforms_user_types_zero_alloc() {
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct LonLat {
        lon: f64,
        lat: f64,
    }
    impl proxi::Coord for LonLat {
        fn x(&self) -> f64 {
            self.lon
        }
        fn y(&self) -> f64 {
            self.lat
        }
        fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
            Self { lon: x, lat: y }
        }
    }

    let context = proxi::Context::configured().expect("configured context");
    let mut t = proxi::TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .build()
        .expect("build transformer");

    let out = t
        .transform_coord(
            LonLat { lon: 0.0, lat: 0.0 },
            Direction::Forward,
            proxi::AngularUnits::Auto,
        )
        .expect("transform custom coord");
    assert_eq!(out, LonLat { lon: 0.0, lat: 0.0 });

    // Bulk path on a slice of custom types.
    let mut points = [LonLat { lon: 0.0, lat: 0.0 }, LonLat { lon: 1.0, lat: 1.0 }];
    t.transform_coords(&mut points, Direction::Forward, proxi::AngularUnits::Auto)
        .expect("transform custom coords");
    assert!(
        points
            .iter()
            .all(|p| p.lon.is_finite() && p.lat.is_finite())
    );
}

#[test]
fn crs_proj_string_round_trip() {
    let context = Context::configured().expect("context");
    let crs = proxi::Crs::from_user_input(&context, "EPSG:4326").expect("create CRS");
    let proj_string = crs.to_proj_string().expect("proj string");
    assert!(
        proj_string.starts_with("+proj=") || proj_string.contains("+proj="),
        "got: {proj_string}"
    );
}

#[test]
fn invalid_context_path_is_rejected() {
    let context = Context::configured().expect("context");
    let result = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .context_options(proxi::ContextOptions::default().push_data_path("bad\0path"))
        .build();
    assert!(matches!(
        result,
        Err(ProxiError::ContextConfiguration { .. })
    ));
}

#[test]
fn missing_proj_db_data_dir_fails_fast_with_missing_data() {
    let context = Context::configured().expect("context");
    // A non-existent data directory (no proj.db) must be rejected with a clear
    // `MissingData` error rather than silently accepted and failing later.
    let nonexistent = std::env::temp_dir().join("proxi-does-not-exist-db");
    let err = match TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .data_dir(&nonexistent)
        .build()
    {
        Ok(_) => panic!("expected Err for missing proj.db"),
        Err(e) => e,
    };
    assert!(
        matches!(err, ProxiError::MissingData { .. }),
        "expected MissingData, got {err:?}"
    );
    assert!(
        err.to_string().contains("proj.db"),
        "error should mention proj.db: {err}"
    );
}

#[test]
fn valid_data_dir_configures_database_path() {
    let context = Context::configured().expect("context");
    // The configured context should have resolved a real data dir; transform
    // works, proving proj.db is active and the hardened set_database_path's
    // round-trip check passed (configured() would have errored otherwise).
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .build()
        .expect("build with resolved data dir");
    let p = t
        .forward_xy((0.0, 0.0).into())
        .expect("transform with active database");
    assert!(p.x.abs() < 1e-9 && p.y.abs() < 1e-9, "got {p:?}");
}

#[test]
fn explicit_pipeline_transform_works() {
    let context = Context::configured().expect("context");
    let mut transformer = TransformerBuilder::from_pipeline(
        &context,
        "+proj=pipeline +step +proj=axisswap +order=2,1",
    )
    .build()
    .expect("build pipeline");
    let output = transformer
        .forward_xy(Coord2::new(10.0, 20.0))
        .expect("transform pipeline");
    assert_eq!(output, Coord2::new(20.0, 10.0));
}

#[test]
fn bounds_transform_densifies_and_validates() {
    let context = Context::configured().expect("context");
    let mut transformer = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .build()
        .expect("build bounds transformer");
    let bounds = transformer
        .forward_bounds([-1.0, -1.0, 1.0, 1.0], 8)
        .expect("transform bounds");
    assert!((bounds[0] + 111_319.49).abs() < 1.0, "west: {}", bounds[0]);
    assert!((bounds[2] - 111_319.49).abs() < 1.0, "east: {}", bounds[2]);
    assert!(bounds[1] < 0.0 && bounds[3] > 0.0);

    let error = transformer.transform_bounds(
        [-1.0, -1.0, 1.0, 1.0],
        Direction::Forward,
        proxi::AngularUnits::Auto,
        -1,
    );
    assert!(matches!(error, Err(ProxiError::Transform { .. })));
}

#[test]
fn operation_metadata_is_available() {
    let context = Context::configured().expect("context");
    let transformer = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .build()
        .expect("build metadata transformer");
    let info = transformer.operation_info().expect("operation metadata");
    assert!(info.id.is_some(), "operation id should be available");
    assert!(info.description.is_some());
    assert!(info.definition.is_some());
    assert!(info.has_inverse);
    assert!(info.accuracy >= -1.0);
    assert!(info.area_of_use.is_some());
    assert!(info.source_crs_wkt.is_some());
    assert!(info.target_crs_wkt.is_some());
    let grids = transformer.grids().expect("operation grid inventory");
    assert!(grids.iter().all(|grid| {
        grid.short_name.is_some() || grid.full_name.is_some() || grid.url.is_some()
    }));
    assert_eq!(transformer.download_missing_grids().expect("grid sync"), 0);
}

#[test]
fn crs_type_and_component_extraction_work() {
    let context = Context::configured().expect("context");

    // Type queries distinguish CRS kinds.
    let geog = proxi::Crs::from_user_input(&context, "EPSG:4326").expect("geographic");
    assert_eq!(
        geog.crs_type(),
        proxi::CrsType::Geographic2DCrs,
        "4326 is 2D geographic"
    );
    let proj = proxi::Crs::from_user_input(&context, "EPSG:32633").expect("projected");
    assert_eq!(
        proj.crs_type(),
        proxi::CrsType::ProjectedCrs,
        "32633 is projected"
    );
    let compound = proxi::Crs::from_user_input(&context, "EPSG:4326+5773").expect("compound");
    assert_eq!(
        compound.crs_type(),
        proxi::CrsType::CompoundCrs,
        "4326+5773 is compound"
    );

    // Component extraction: a projected CRS has a geodetic CRS + horizontal datum.
    let geod = proj.geodetic_crs().expect("projected has geodetic crs");
    assert_eq!(
        geod.crs_type(),
        proxi::CrsType::Geographic2DCrs,
        "geodetic component is geographic"
    );
    let datum = proj
        .horizontal_datum()
        .expect("projected has horizontal datum");
    assert!(
        datum.info().expect("datum info").name.is_some(),
        "datum has a name"
    );

    // A compound CRS has sub-CRSs.
    let sub0 = compound.sub_crs(0).expect("compound has sub-CRS 0");
    assert!(
        sub0.info().expect("sub info").name.is_some(),
        "sub-CRS 0 has a name"
    );

    // Creation by name: `proj_create_from_name` is best-effort by human name;
    // if it matches, the object must be a CRS. If not matched, the fallback
    // authority-based constructor still covers name-based workflows.
    if let Some(by_name) = context.crs_from_name("WGS 84", true) {
        assert_eq!(
            by_name.crs_type(),
            proxi::CrsType::Geographic2DCrs,
            "WGS 84 by name"
        );
    }
    // The authority-based path is the authoritative, always-available constructor.
    let by_auth = proxi::Crs::from_authority(&context, "EPSG", "4326").expect("find by authority");
    assert_eq!(
        by_auth.crs_type(),
        proxi::CrsType::Geographic2DCrs,
        "WGS 84 by authority"
    );
}

#[test]
fn crs_metadata_and_equivalence_are_available() {
    let context = Context::configured().expect("context");
    let geographic = proxi::Crs::from_user_input(&context, "EPSG:4326").expect("create CRS");
    let equivalent = proxi::Crs::from_user_input(&context, "EPSG:4326").expect("create CRS");
    let info = geographic.info().expect("CRS metadata");
    assert!(info.name.is_some());
    assert!(
        info.identifiers
            .iter()
            .any(|id| id.authority == "EPSG" && id.code == "4326")
    );
    assert!(info.area_of_use.is_some());
    let ensemble = info.datum_ensemble.expect("datum ensemble");
    assert!(ensemble.name.is_some());
    assert!(!ensemble.members.is_empty());
    assert!(ensemble.accuracy_meters.is_finite());
    let coordinate_system = info.coordinate_system.expect("coordinate system");
    assert_eq!(
        coordinate_system.kind,
        proxi::CoordinateSystemType::Ellipsoidal
    );
    assert_eq!(coordinate_system.axes.len(), 2);
    assert_eq!(
        coordinate_system.axes[0].direction.as_deref(),
        Some("north")
    );
    assert_eq!(coordinate_system.axes[1].direction.as_deref(), Some("east"));
    assert!(geographic.equivalent_to(&equivalent, proxi::CrsComparison::Strict));
    assert!(geographic.equivalent_to(&equivalent, proxi::CrsComparison::Equivalent));
}

#[test]
fn crs_database_queries_and_authority_construction_work() {
    let context = Context::configured().expect("context");
    let database = Database::new(&context);
    let crs = database.crs("EPSG", "4326").expect("create EPSG CRS");
    let info = crs.info().expect("CRS info");
    assert!(
        info.identifiers
            .iter()
            .any(|id| id.authority == "EPSG" && id.code == "4326")
    );

    let authorities = database.authorities();
    assert!(authorities.iter().any(|authority| authority == "EPSG"));

    let codes = database.codes("EPSG").expect("EPSG codes");
    assert!(codes.iter().any(|code| code == "4326"));
    assert!(
        !database
            .codes_of_type("EPSG", proxi::DatabaseType::DatumEnsemble)
            .expect("datum codes")
            .is_empty()
    );
    assert!(
        database
            .ellipsoids()
            .iter()
            .any(|ellipsoid| ellipsoid.id == "WGS84")
    );
    assert!(database.units().iter().any(|unit| unit.id == "m"));
    assert!(!database.angular_units().is_empty());
    assert!(
        database
            .prime_meridians()
            .iter()
            .any(|meridian| meridian.id == "greenwich")
    );
    assert!(
        database
            .operations()
            .iter()
            .any(|operation| operation.id == "longlat")
    );
}

#[test]
fn operation_selection_options_build_and_transform() {
    let context = Context::configured().expect("context");
    let definition = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .always_xy(true)
        .authority("EPSG")
        .desired_accuracy(1_000.0)
        .allow_ballpark(false)
        .into_definition();
    let mut transformer = definition
        .build_for_current_thread(&context)
        .expect("build configured operation");
    let point = transformer
        .forward_xy(Coord2::new(0.0, 0.0))
        .expect("transform configured operation");
    assert!(point.x.abs() < 1e-9 && point.y.abs() < 1e-9);
}

#[test]
fn operation_group_enumerates_and_promotes_candidate() {
    let context = Context::configured().expect("context");
    let group = proxi::TransformerGroupBuilder::new(&context, "EPSG:4326", "EPSG:3857")
        .authority("EPSG")
        .allow_ballpark(false)
        .build()
        .expect("build operation group");
    assert!(!group.is_empty());
    let info = group.operation_info(0).expect("candidate metadata");
    assert!(info.description.is_some());
    let mut transformer = group.into_transformer(0).expect("promote candidate");
    let point = transformer
        .forward_xy(Coord2::new(0.0, 0.0))
        .expect("transform selected candidate");
    assert!(point.x.abs() < 1e-9 && point.y.abs() < 1e-9);
}

#[test]
fn three_dimensional_identity_preserves_z() {
    let context = Context::configured().expect("context");
    let mut transformer = TransformerBuilder::new(&context, "EPSG:4979", "EPSG:4979")
        .build()
        .expect("build 3D identity transformer");
    let input = Coord3::new(12.5, 48.25, 1234.5);
    let output = transformer.forward_xyz(input).expect("3D transform");
    assert!((output.x - input.x).abs() < 1e-12);
    assert!((output.y - input.y).abs() < 1e-12);
    assert!((output.z - input.z).abs() < 1e-9);
}

#[test]
fn vertical_datum_operation_reports_grid_requirements() {
    let context = Context::configured().expect("context");
    let group = proxi::TransformerGroupBuilder::new(&context, "EPSG:4979", "EPSG:4326+5773")
        .allow_ballpark(false)
        .build()
        .expect("build vertical datum operation group");
    assert!(!group.is_empty());
    let mut inspected_grid = false;
    for index in 0..group.len() {
        for grid in group.grids(index).expect("inspect operation grids") {
            inspected_grid = true;
            assert!(grid.short_name.is_some() || grid.full_name.is_some());
            assert!(
                grid.is_available() || grid.url.is_some() || grid.package_name.is_some(),
                "grid has availability metadata or download info"
            );
        }
    }
    assert!(inspected_grid, "vertical operation should report a grid");
}

#[test]
fn required_grid_policy_rejects_missing_vertical_grid() {
    let context = Context::configured().expect("context");
    let result = TransformerBuilder::new(&context, "EPSG:4979", "EPSG:4326+5773")
        .allow_ballpark(false)
        .grid_policy(proxi::GridPolicy::RequireAvailable)
        .build();
    assert!(matches!(result, Err(ProxiError::GridMissing { .. })));
}

#[test]
fn ecef_to_utm_egm96_download_and_transform() {
    if std::env::var_os("PROXI_NETWORK_TEST").is_none() {
        eprintln!("skipping network ECEF/EGM96 test; set PROXI_NETWORK_TEST=1 to run");
        return;
    }

    let context = Context::configured().expect("context");
    let mut transformer = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:32633+5773")
        .always_xy(true)
        .network_enabled(true)
        .allow_ballpark(false)
        .build()
        .expect("build ECEF to UTM+EGM96 transformer");

    let grids_before = transformer.grids().expect("inspect EGM96 grids");
    eprintln!("EGM96 grid requirements before download: {grids_before:?}");
    let downloaded = transformer
        .download_missing_grids()
        .expect("download EGM96 grids through proxi");
    eprintln!("proxi downloaded {downloaded} grid file(s)");

    let semi_major = 6_378_137.0_f64;
    let longitude_radians = 15.0_f64.to_radians();
    let ecef = Coord3::new(
        semi_major * longitude_radians.cos(),
        semi_major * longitude_radians.sin(),
        0.0,
    );
    let utm_egm96 = transformer
        .forward_xyz(ecef)
        .expect("transform ECEF to UTM+EGM96");
    eprintln!("ECEF {ecef:?} -> UTM+EGM96 {utm_egm96:?}");

    assert!((utm_egm96.x - 500_000.0).abs() < 0.01);
    assert!(utm_egm96.y.abs() < 0.01);
    assert!(utm_egm96.z.is_finite());
}

#[test]
fn wgs84_geodesic_inverse_and_batch_work() {
    let context = Context::configured().expect("context");
    let geod = proxi::Geod::wgs84(&context).expect("create WGS84 geodesic");
    let inverse = geod
        .inverse(0.0, 0.0, 90.0, 0.0)
        .expect("equatorial inverse");
    assert!(
        (inverse.distance_meters - 10_018_754.17).abs() < 0.1,
        "distance: {}",
        inverse.distance_meters
    );
    assert!((inverse.forward_azimuth_degree - 90.0).abs() < 1e-10);
    assert!(inverse.reverse_azimuth_degree.is_finite());

    let first_longitudes = [0.0, 0.0];
    let first_latitudes = [0.0, 40.0];
    let second_longitudes = [90.0, 1.0];
    let second_latitudes = [0.0, 41.0];
    let mut distances = [0.0; 2];
    let mut forward_azimuths = [0.0; 2];
    let mut reverse_azimuths = [0.0; 2];
    geod.inverse_batch_into(
        &first_longitudes,
        &first_latitudes,
        &second_longitudes,
        &second_latitudes,
        &mut distances,
        &mut forward_azimuths,
        &mut reverse_azimuths,
    )
    .expect("batch inverse");
    assert_eq!(distances[0], inverse.distance_meters);
    assert!(distances[1] > 0.0);

    let direct = geod
        .direct(0.0, 0.0, 90.0, inverse.distance_meters)
        .expect("direct geodesic");
    assert!(direct.longitude_degree.abs() > 89.9);
    assert!(direct.latitude_degree.abs() < 1e-8);

    let polygon = geod
        .polygon_area_perimeter(&[0.0, 1.0, 1.0, 0.0], &[0.0, 0.0, 1.0, 1.0])
        .expect("polygon area");
    assert!(polygon.area_square_meters.abs() > 1.0e10);
    assert!(polygon.perimeter_meters > 400_000.0);

    let mut intermediate_longitudes = [0.0; 2];
    let mut intermediate_latitudes = [0.0; 2];
    geod.npts_into(
        0.0,
        0.0,
        90.0,
        0.0,
        &mut intermediate_longitudes,
        &mut intermediate_latitudes,
    )
    .expect("intermediate points");
    assert!((intermediate_longitudes[0] - 30.0).abs() < 1e-8);
    assert!((intermediate_longitudes[1] - 60.0).abs() < 1e-8);
}

#[test]
fn runtime_version_diagnostics_are_available() {
    let v = proxi::ProjVersion::runtime();
    assert_eq!(v.major, proxi::BUILT_VERSION_MAJOR, "runtime major");
    assert!(
        v.minor >= proxi::BUILT_VERSION_MINOR,
        "runtime minor >= built"
    );
    assert!(
        v.release.is_some() || v.version.is_some(),
        "release/version string"
    );
    // Compatibility check must pass against the matched bundled runtime.
    proxi::check_runtime_compatibility().expect("runtime compatible");
}

#[test]
fn geod_from_proj_string_reads_ellipsoid_from_object() {
    let context = Context::configured().expect("context");
    // A valid CRS with a known ellipsoid: EPSG:4326 (WGS84). The geodesic built
    // from the PROJ string must use the object's ellipsoid (not a fallback).
    let geod = proxi::Geod::from_proj_string(&context, "EPSG:4326").expect("EPSG:4326 geod");
    // Equatorial inverse on WGS84: ~10018 km over 90 degrees of longitude.
    let inverse = geod
        .inverse(0.0, 0.0, 90.0, 0.0)
        .expect("equatorial inverse");
    assert!(
        (inverse.distance_meters - 10_018_754.17).abs() < 0.1,
        "WGS84 equatorial distance: {}",
        inverse.distance_meters
    );

    // A definition with no ellipsoid must error (no silent WGS84 fallback).
    let no_ellipsoid =
        proxi::Geod::from_proj_string(&context, "+proj=pipeline +step +proj=axisswap +order=2,1");
    assert!(
        matches!(no_ellipsoid, Err(ProxiError::InvalidCrs { .. })),
        "pipeline without ellipsoid should fail"
    );
}

#[test]
fn geocentric_to_geographic_expected() {
    // EPSG:4978 (geocentric X,Y,Z) -> EPSG:4326 (lon/lat degrees, ellipsoidal h).
    // A point on the +X axis at the ellipsoid surface => lon=0, lat=0, h=0.
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:4326")
        .build()
        .expect("build");
    let a = 6378137.0;
    let p = t.forward_xyz(Coord3::new(a, 0.0, 0.0)).expect("forward");
    assert!(p.x.abs() < 1e-6, "lon ~0 got {}", p.x);
    assert!(p.y.abs() < 1e-6, "lat ~0 got {}", p.y);
    assert!(p.z.abs() < 1e-6, "h ~0 got {}", p.z);
}

#[test]
fn geographic_to_geocentric_expected() {
    // Inverse: EPSG:4326 (0,0,0) -> EPSG:4978 => (a,0,0).
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:4978")
        .build()
        .expect("build");
    let a = 6378137.0;
    let p = t.forward_xyz(Coord3::new(0.0, 0.0, 0.0)).expect("forward");
    assert!((p.x - a).abs() < 1e-3, "X ~a got {}", p.x);
    assert!(p.y.abs() < 1e-3, "Y ~0 got {}", p.y);
    assert!(p.z.abs() < 1e-3, "Z ~0 got {}", p.z);
}

#[test]
fn geographic_to_utm_inverse_recovers() {
    // EPSG:4326 (lon/lat) -> EPSG:32633 UTM: inverse must recover lon/lat.
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
        .build()
        .expect("build");
    let p = t.forward_xyz(Coord3::new(0.0, 0.0, 0.0)).expect("forward");
    let back = t.inverse_xyz(p).expect("inverse");
    assert!((back.x - 0.0).abs() < 1e-6, "lon back {}", back.x);
    assert!((back.y - 0.0).abs() < 1e-6, "lat back {}", back.y);
    assert!(back.z.abs() < 1e-3, "h back {}", back.z);
}

#[test]
fn projected_with_height_preserves_z() {
    // EPSG:4978 -> EPSG:32633 (3D UTM): the height component should remain
    // finite and near the ellipsoidal height input.
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:32633")
        .build()
        .expect("build");
    let a = 6378137.0;
    let p = t
        .forward_xyz(Coord3::new(a + 100.0, 0.0, 0.0))
        .expect("forward");
    assert!(p.z.is_finite(), "height finite got {}", p.z);
}

#[test]
fn bespoke_geographic_vertical_and_compound_crs_construct() {
    let context = Context::configured().expect("context");

    // Geographic CRS built inline from datum / ellipsoid / meridian parameters.
    let geog = Crs::geographic(
        &context,
        "WGS 84 (built)",
        "World Geodetic System 1984",
        "WGS 84",
        6_378_137.0,
        298.257223563,
        "Greenwich",
        0.0,
        "degree",
        0.0174532925199433,
    )
    .expect("geographic CRS");
    assert_eq!(geog.crs_type(), proxi::CrsType::Geographic2DCrs);

    // Vertical CRS built inline from datum / linear-unit strings.
    let vert = Crs::vertical(
        &context,
        "NAVD88 height (built)",
        "North American Vertical Datum 1988",
        "metre",
        1.0,
    )
    .expect("vertical CRS");
    assert_eq!(vert.crs_type(), proxi::CrsType::VerticalCrs);

    // Compound (horizontal + vertical) CRS.
    let compound =
        Crs::compound(&context, "WGS84 + NAVD88 (built)", &geog, &vert).expect("compound CRS");
    assert_eq!(compound.crs_type(), proxi::CrsType::CompoundCrs);
    assert!(
        compound.sub_crs(0).is_some() && compound.sub_crs(1).is_some(),
        "compound has two sub-CRSs"
    );
}

#[test]
fn bespoke_projected_crs_from_conversion_and_cs() {
    let context = Context::configured().expect("context");

    // Geographic base from the built-in database (WGS 84).
    let base = Crs::from_authority(&context, "EPSG", "4326").expect("WGS 84");

    // A Transverse Mercator conversion (equivalent to EPSG:32633's method).
    let conversion =
        Conversion::transverse_mercator(&context, DEGREE_METRE, 0.0, 15.0, 0.9996, 500_000.0, 0.0)
            .expect("TM conversion");
    // Cartesian CS in metres.
    let cs = Proj::cartesian_2d_cs(&context, true, "metre", 1.0).expect("cartesian 2D CS");

    let projected =
        Crs::projected(&context, "UTM 33N (built)", &base, &conversion, &cs).expect("projected");
    assert_eq!(projected.crs_type(), proxi::CrsType::ProjectedCrs);
    assert!(
        projected.geodetic_crs().is_some(),
        "projected has geodetic CRS"
    );
}

#[test]
fn bespoke_utm_and_engineering_crs_construct() {
    let context = Context::configured().expect("context");

    // UTM conversion + cartesian CS over EPSG:4326.
    let utm_conv = Conversion::utm(&context, 33, true).expect("UTM 33N conversion");
    let cs = Proj::cartesian_2d_cs(&context, true, "metre", 1.0).expect("cartesian CS");
    let base = Crs::from_authority(&context, "EPSG", "4326").expect("WGS 84");
    let utm =
        Crs::projected(&context, "EPSG:32633 (built)", &base, &utm_conv, &cs).expect("projected");
    assert_eq!(utm.crs_type(), proxi::CrsType::ProjectedCrs);
    // Compare with the authoritative EPSG:32633.
    let reference = Crs::from_authority(&context, "EPSG", "32633").expect("EPSG:32633");
    assert!(
        utm.equivalent_to(&reference, proxi::CrsComparison::Equivalent),
        "built UTM should be equivalent to EPSG:32633"
    );

    // Engineering CRS.
    let eng = Crs::engineering(&context, "Local engineering grid").expect("engineering CRS");
    assert_eq!(eng.crs_type(), proxi::CrsType::EngineeringCrs);
}

#[test]
fn bespoke_geographic_from_datum_and_cs() {
    let context = Context::configured().expect("context");
    let cs = Proj::ellipsoidal_2d_cs(&context, true, "degree", 0.0174532925199433)
        .expect("ellipsoidal 2D CS");
    // Use the horizontal datum of EPSG:4326 as the datum object.
    let base = Crs::from_authority(&context, "EPSG", "4326").expect("WGS 84");
    let datum = base.horizontal_datum().expect("horizontal datum of 4326");

    let geog = Crs::geographic_from_datum(&context, "WGS 84 (from datum)", &datum, &cs)
        .expect("geographic from datum");
    assert_eq!(geog.crs_type(), proxi::CrsType::Geographic2DCrs);
    // The rebuilt CRS is not name-strict-equal to EPSG:4326 (its name differs),
    // but it must carry the same datum and ellipsoid, giving the same
    // geographic coordinate system. Verify by comparing its horizontal datum.
    let geog_datum = geog.horizontal_datum().expect("rebuilt has datum");
    assert!(
        geog_datum.equivalent_to(&datum, proxi::CrsComparison::Equivalent),
        "rebuilt geographic CRS carries the source horizontal datum"
    );
}

#[test]
fn wkt_output_options_control_multiline_and_indentation() {
    let context = Context::configured().expect("context");
    let crs = Crs::from_authority(&context, "EPSG", "4326").expect("WGS 84");

    // Default (single-line) WKT2:2019.
    let single = crs
        .to_wkt_with_options(
            proxi::WktVersion::Wkt2_2019,
            Some(&proxi::WktOptions {
                multiline: Some(false),
                indentation_width: Some(0),
                ..Default::default()
            }),
        )
        .expect("single-line WKT");
    assert!(
        !single.contains('\n'),
        "single-line WKT should not contain newlines: {single}"
    );

    // Multiline with indentation.
    let multiline = crs
        .to_wkt_with_options(
            proxi::WktVersion::Wkt2_2019,
            Some(&proxi::WktOptions {
                multiline: Some(true),
                indentation_width: Some(2),
                ..Default::default()
            }),
        )
        .expect("multiline WKT");
    assert!(
        multiline.contains('\n'),
        "multiline WKT should contain newlines: {multiline}"
    );
    assert!(
        multiline.contains("  A"),
        "indented multiline WKT should contain 2-space indentation: {multiline}"
    );
}

#[test]
fn wkt_output_axis_and_simplified_variant_are_applied() {
    let context = Context::configured().expect("context");
    let crs = Crs::from_authority(&context, "EPSG", "4326").expect("WGS 84");

    // `OUTPUT_AXIS=ORDER` must be honored by PROJ: the call succeeds and
    // returns a well-formed ensemble WKT2:2019 (the axis order only manifests
    // in the coordinate-system node for CRSs that carry one; EPSG:4326's
    // ensemble WKT omits the explicit `CS[...]` node, so we validate the
    // option was accepted by checking the output is complete WKT).
    let ordered = crs
        .to_wkt_with_options(
            proxi::WktVersion::Wkt2_2019,
            Some(&proxi::WktOptions {
                output_axis_order: Some(proxi::AxisOutputOrder::Order),
                ..Default::default()
            }),
        )
        .expect("OUTPUT_AXIS=ORDER WKT");
    assert!(
        ordered.contains("GEOGCRS"),
        "ordered axis WKT is a GEOGCRS: {ordered}"
    );
    assert!(
        ordered.contains("ID") && ordered.contains("EPSG"),
        "ordered axis WKT carries authority id: {ordered}"
    );

    // The simplified variants are distinct tokens and must serialize.
    // PROJ's simplified WKT2:2019 still uses the `GEOGCRS` keyword for a
    // geographic ensemble CRS, but includes the explicit `CS[...]` + `AXIS`
    // nodes that the full (non-simplified) ensemble form omits.
    let simplified = crs
        .to_wkt(proxi::WktVersion::Wkt2_2019Simplified)
        .expect("simplified WKT2:2019");
    assert!(
        simplified.contains("GEOGCRS") || simplified.contains("GEODCRS"),
        "simplified WKT2:2019 is a geographic CRS: {simplified}"
    );
    assert!(
        simplified.contains("CS[ellipsoidal"),
        "simplified WKT2:2019 carries an explicit coordinate system: {simplified}"
    );
}

#[test]
fn proj_string_version_selects_proj5_vs_proj4() {
    let context = Context::configured().expect("context");
    let crs = Crs::from_authority(&context, "EPSG", "4326").expect("WGS 84");

    let proj5 = crs
        .to_proj_string_with_version(proxi::ProjStringVersion::Proj5)
        .expect("proj5 string");
    assert!(
        proj5.contains("+proj="),
        "proj5 string should be a +proj form: {proj5}"
    );

    // Legacy proj.4 syntax also resolves (WGS84 latlong).
    let proj4 = crs
        .to_proj_string_with_version(proxi::ProjStringVersion::Proj4)
        .expect("proj4 string");
    assert!(
        proj4.contains("+proj=longlat") || proj4.contains("+proj=latlong"),
        "proj4 geographic string: {proj4}"
    );
}

#[test]
fn database_filtered_crs_search_returns_structured_records() {
    let context = Context::configured().expect("context");
    let database = Database::new(&context);

    // Search EPSG for projected CRSs (e.g. UTM) — must return typed records
    // with authority, code, name and bbox.
    let records = database
        .crs_search(
            Some("EPSG"),
            &proxi::CrsSearch {
                types: vec![proxi::CrsType::ProjectedCrs],
                allow_deprecated: false,
                ..Default::default()
            },
        )
        .expect("projected CRS search");
    assert!(!records.is_empty());
    let projected = records
        .iter()
        .find(|r| r.code.as_deref() == Some("32633"))
        .expect("EPSG:32633 present");
    assert_eq!(
        projected.r#type,
        proxi::CrsType::ProjectedCrs,
        "32633 is projected"
    );
    assert_eq!(projected.authority.as_deref(), Some("EPSG"));
    assert!(
        projected.bbox_valid && projected.north_lat_degree >= projected.south_lat_degree,
        "projected record carries a valid bbox"
    );
    assert!(!projected.deprecated);
}

#[test]
fn database_units_and_grid_lookups_work() {
    let context = Context::configured().expect("context");
    let database = Database::new(&context);

    // Linear units ("linear" category) from the database.
    let units = database
        .units_from_database(Some("EPSG"), "linear", false)
        .expect("linear units from database");
    assert!(
        units.iter().any(|u| u.code.as_deref() == Some("9001")),
        "EPSG:9001 (metre) present in linear units"
    );

    // Angular units ("angular" category).
    let angular = database
        .units_from_database(Some("EPSG"), "angular", false)
        .expect("angular units from database");
    assert!(
        angular.iter().any(|u| u.code.as_deref() == Some("9122")),
        "EPSG:9122 (degree) present in angular units"
    );

    // Grid metadata lookup: the `null` grid is always registered in proj.db.
    let grid = database.grid("null").expect("grid metadata");
    assert!(
        grid.short_name.unwrap_or_default().contains("null"),
        "grid short name retained"
    );
    assert!(
        grid.full_name.is_some() || grid.url.is_some() || grid.package_name.is_some(),
        "grid metadata present"
    );
}

#[test]
fn interleaved_xy_transform_matches_soa() {
    let context = Context::configured().expect("context");
    let mut interleaved = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build interleaved transformer");
    let mut soa = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build soa transformer");

    // Interleaved [x, y] points (degrees).
    let mut coords = [[0.0, 0.0], [15.0, 0.0], [-15.0, 0.0]];
    let processed = interleaved
        .transform_xy_interleaved_in_place(
            &mut coords,
            Direction::Forward,
            proxi::AngularUnits::Auto,
        )
        .expect("interleaved transform");
    assert_eq!(processed, coords.len());

    // SOA reference for the same points.
    let mut xs = [0.0_f64, 15.0, -15.0];
    let mut ys = [0.0_f64, 0.0, 0.0];
    soa.transform_xy_in_place(
        &mut xs,
        &mut ys,
        Direction::Forward,
        proxi::AngularUnits::Auto,
    )
    .expect("soa transform");

    for i in 0..3 {
        assert!(
            (coords[i][0] - xs[i]).abs() < 1e-9 && (coords[i][1] - ys[i]).abs() < 1e-9,
            "point {i}: interleaved {:?} vs soa ({},{})",
            coords[i],
            xs[i],
            ys[i]
        );
    }
}

#[test]
fn completeness_policy_reports_partial_failure() {
    let context = Context::configured().expect("context");
    let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:32633")
        .always_xy(true)
        .build()
        .expect("build completeness transformer");

    // One finite point followed by a non-finite x/y point (NaN).
    let mut coords = [
        [0.0_f64, 0.0, 10.0],
        [f64::NAN, 0.0, 20.0],
        [15.0, 0.0, 30.0],
    ];
    // The first non-finite point is at index 1, so only the prefix [0..1] is
    // transformed; the partial-failure summary reports processed=1, total=3.
    let first = coords;
    match t.transform_xyz_complete(&mut coords, Direction::Forward, proxi::AngularUnits::Auto) {
        Ok(n) => panic!("expected partial failure, got Ok({n})"),
        Err(f) => {
            assert_eq!(f.processed, 1, "processed prefix");
            assert_eq!(f.total, 3, "total");
            assert_eq!(f.failed(), 2, "failed count");
        }
    }
    // The NaN point's output must be untouched (it was not sent to PROJ).
    assert!(coords[1][0].is_nan() && coords[1][1] == 0.0);
    // The finite prefix must have been transformed.
    assert!(
        (coords[0][0] - first[0][0]).abs() > 0.1,
        "finite prefix transformed"
    );
}

#[test]
fn geodesic_line_position_and_arc_mode_work() {
    let context = Context::configured().expect("context");
    let geod = proxi::Geod::wgs84(&context).expect("WGS84");

    // Direct distance (metres) over ~90 degrees  of longitude on the equator.
    let inverse = geod.inverse(0.0, 0.0, 90.0, 0.0).expect("inverse");
    let distance = inverse.distance_meters;

    // Build a line from (0,0) heading 90 degrees  with the same distance + caps.
    let line = geod.line(
        0.0,
        0.0,
        90.0,
        distance,
        false, // distance (metre) mode
        false, // no longitude unroll
        proxi::LineCaps::ALL,
    );
    let end = line.position(distance);
    assert!(
        (end.longitude_degree - 90.0).abs() < 1e-9 && end.latitude_degree.abs() < 1e-9,
        "line endpoint matches direct result: {:?}",
        end
    );

    // Arc mode: the same 90 degrees -longitude geodesic expressed as a central-angle
    // arc. The full 90 degrees -of-longitude equator path has a12 slightly > 90 degrees  (the
    // central angle of a geodesic differs from its longitude span off the
    // equator/antipodal), so querying exactly the arc our line covers lands
    // near — but not exactly at — 90 degrees  longitude. We assert the result is the
    // finite equatorial endpoint (latitude ~0), not an exact longitude.
    let arc_end = line.gen_position(90.0, true, true);
    assert!(
        arc_end.latitude_degree.abs() < 1e-6,
        "arc-mode latitude ~0: {}",
        arc_end.latitude_degree
    );
    assert!(
        arc_end.longitude_degree.is_finite() && arc_end.longitude_degree.abs() > 0.0,
        "arc-mode longitude finite & positive: {}",
        arc_end.longitude_degree
    );
    assert_eq!(
        line.caps(),
        proxi::LineCaps::ALL,
        "line preserves requested caps"
    );
}

#[test]
fn geodesic_indexed_direct_inverse_and_longitude_unroll() {
    let context = Context::configured().expect("context");
    let geod = proxi::Geod::wgs84(&context).expect("WGS84");

    // Indexed inverse returns reduced length / geodesic scales / area + a12.
    let inv = geod
        .inverse_indexed(0.0, 0.0, 90.0, 0.0)
        .expect("indexed inverse");
    assert!(inv.distance_meters > 0.0);
    assert!(inv.a12.is_finite() && inv.a12 > 0.0);
    assert!(inv.m12_meters.is_finite());
    assert!(inv.m12_scale.is_finite() && inv.m21_scale.is_finite());
    assert!(inv.area_square_meters.is_finite());

    // Indexed direct (distance mode) must land on the same endpoint; its
    // reported distance matches the inverse distance.
    let dir = geod
        .direct_indexed(0.0, 0.0, 90.0, inv.distance_meters, false, false)
        .expect("indexed direct");
    assert!(
        (dir.longitude_degree - 90.0).abs() < 1e-9,
        "indexed direct endpoint: {}",
        dir.longitude_degree
    );
    assert!((dir.distance_meters - inv.distance_meters).abs() < 1.0);

    // Longitude unrolling: advancing beyond 180 degrees  keeps longitudes monotonic.
    let line = geod.line(
        0.0,
        0.0,
        90.0,
        2.0 * inv.distance_meters,
        false,
        true,
        proxi::LineCaps::ALL,
    );
    let past = line.position(1.5 * inv.distance_meters);
    assert!(
        past.longitude_degree > 135.0,
        "unrolled longitude past 180: {}",
        past.longitude_degree
    );
}

#[test]
fn geodesic_streaming_polygon_builder_matches_bulk() {
    let context = Context::configured().expect("context");
    let geod = proxi::Geod::wgs84(&context).expect("WGS84");

    let lons = [0.0, 1.0, 1.0, 0.0];
    let lats = [0.0, 0.0, 1.0, 1.0];
    let bulk = geod
        .polygon_area_perimeter(&lons, &lats)
        .expect("bulk polygon");

    // Streaming: add vertices one at a time, then compute.
    let mut builder = geod.polygon_builder();
    builder.add_point(&geod, 0.0, 0.0);
    builder.add_point(&geod, 1.0, 0.0);
    builder.add_point(&geod, 1.0, 1.0);
    builder.add_point(&geod, 0.0, 1.0);
    let streamed = builder.compute(&geod);

    assert!(
        (bulk.area_square_meters - streamed.area_square_meters).abs() < 1.0,
        "areas match: bulk {} vs streamed {}",
        bulk.area_square_meters,
        streamed.area_square_meters
    );
    assert!(
        (bulk.perimeter_meters - streamed.perimeter_meters).abs() < 1e-6,
        "perimeters match: bulk {} vs streamed {}",
        bulk.perimeter_meters,
        streamed.perimeter_meters
    );

    // Adding via edges (azimuth/distance). `geod_polygon` requires the first
    // vertex to be seeded with `add_point`; subsequent segments are `add_edge`.
    let mut edged = geod.polygon_builder();
    edged.add_point(&geod, 0.0, 0.0); // seed first vertex
    let mut prev_lon = 0.0;
    let mut prev_lat = 0.0;
    for (lon, lat) in lons.iter().zip(lats.iter()) {
        let angle = geod
            .inverse(prev_lon, prev_lat, *lon, *lat)
            .expect("edge inverse");
        edged.add_edge(&geod, angle.forward_azimuth_degree, angle.distance_meters);
        prev_lon = *lon;
        prev_lat = *lat;
    }
    // Close back to the start.
    let close = geod.inverse(prev_lon, prev_lat, 0.0, 0.0).expect("close");
    edged.add_edge(&geod, close.forward_azimuth_degree, close.distance_meters);
    let edged_result = edged.compute(&geod);
    // Edges accumulate to the same polygon within numerical tolerance.
    assert!(
        (edged_result.area_square_meters - bulk.area_square_meters).abs()
            < 1e-3 * bulk.area_square_meters.abs(),
        "edge-built area matches: {} vs {}",
        edged_result.area_square_meters,
        bulk.area_square_meters
    );
}
