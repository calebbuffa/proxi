use proxi::{Context, Coord3, GridPolicy, Result, TransformerBuilder};

fn main() -> Result<()> {
    let context = Context::new()?;
    println!("Data Paths: {:?}", context.data_paths());

    // WGS84 ECEF -> UTM zone 33N with the EGM96 geoid height correction.
    let mut transformer = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:32633+5773")
        .always_xy(true)
        .allow_ballpark(false)
        .grid_policy(GridPolicy::DownloadMissing)
        .build()?;

    println!("Required grids:");
    for grid in transformer.grids()? {
        println!("  {grid:?}");
    }

    // A realistic WGS84 point at longitude 15 degrees east, latitude 40 degrees
    // north, and 10 m ellipsoidal height. EGM96 converts that ellipsoidal
    // height to an orthometric height in the output vertical CRS.
    let longitude_radians = 15.0_f64.to_radians();
    let latitude_radians = 40.0_f64.to_radians();
    let ellipsoidal_height_meters = 10.0;
    let semi_major_meters = 6_378_137.0;
    let inverse_flattening = 298.257_223_563;
    let flattening = 1.0 / inverse_flattening;
    let eccentricity_squared = flattening * (2.0 - flattening);
    let prime_vertical_radius =
        semi_major_meters / (1.0 - eccentricity_squared * latitude_radians.sin().powi(2)).sqrt();
    let ecef = Coord3::new(
        (prime_vertical_radius + ellipsoidal_height_meters)
            * latitude_radians.cos()
            * longitude_radians.cos(),
        (prime_vertical_radius + ellipsoidal_height_meters)
            * latitude_radians.cos()
            * longitude_radians.sin(),
        (prime_vertical_radius * (1.0 - eccentricity_squared) + ellipsoidal_height_meters)
            * latitude_radians.sin(),
    );
    let utm_egm96 = transformer.forward_xyz(ecef)?;

    println!("ECEF:      {ecef:?}");
    println!("UTM+EGM96: {utm_egm96:?}");
    Ok(())
}
