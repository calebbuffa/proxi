//! Options controlling transformer construction and transform calls.

/// The direction of a coordinate operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Forward (source CRS to target CRS).
    Forward,
    /// Inverse (target CRS back to source CRS).
    Inverse,
}

/// How angular (degree/radian) units are handled for a transform call.
///
/// PROJ expects angles in radians for angular CRSs and does not automatic-
/// ally convert. `Auto` mirrors pyproj: it inspects the operation's input /
/// output unit expectations (via `proj_angular_input` / `proj_degree_input`)
/// and converts `x`/`y` accordingly. `Degrees` and `Radians` force a specific
/// interpretation and assume PROJ's own convention for the matching output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngularUnits {
    /// Inspect the operation and convert degrees <-> radians automatically.
    Auto,
    /// The input/output `x`,`y` are in degrees (angular CRS) and should be
    /// converted to radians for input and from radians for output.
    Degrees,
    /// The input/output `x`,`y` are already in radians.
    Radians,
}

/// A rectangular area of interest, in degrees, used when creating a
/// coordinate operation between two CRSs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaOfInterest {
    /// Western-most longitude, in degrees.
    pub west_lon_degree: f64,
    /// Southern-most latitude, in degrees.
    pub south_lat_degree: f64,
    /// Eastern-most longitude, in degrees.
    pub east_lon_degree: f64,
    /// Northern-most latitude, in degrees.
    pub north_lat_degree: f64,
}

/// Geographic extent associated with a CRS or coordinate operation.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaOfUse {
    pub west_lon_degree: f64,
    pub south_lat_degree: f64,
    pub east_lon_degree: f64,
    pub north_lat_degree: f64,
    pub name: Option<String>,
}

impl AreaOfInterest {
    pub fn new(
        west_lon_degree: f64,
        south_lat_degree: f64,
        east_lon_degree: f64,
        north_lat_degree: f64,
    ) -> Self {
        Self {
            west_lon_degree,
            south_lat_degree,
            east_lon_degree,
            north_lat_degree,
        }
    }
}

/// Configuration for a PROJ context, applied before any CRS or transformer
/// object is created. Mirrors pyproj's context configuration.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ContextOptions {
    /// Explicit path to the `proj.db` file. This takes precedence over all
    /// environment and compiled-in defaults.
    pub database_path: Option<std::path::PathBuf>,
    /// Data search paths (directories containing `proj.db` and grids).
    pub data_paths: Vec<std::path::PathBuf>,
    /// User-writable directory for downloaded grids. Defaults to PROJ's
    /// user-writable directory when `None`.
    pub user_data_dir: Option<std::path::PathBuf>,
    /// Whether to allow network grid downloads. Defaults to `false` (off).
    pub network_enabled: bool,
    /// CA bundle path used for HTTPS network requests.
    pub ca_bundle_path: Option<std::path::PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridPolicy {
    #[default]
    AllowMissing,
    RequireAvailable,
    DownloadMissing,
}

impl ContextOptions {
    /// Set the explicit `proj.db` path for this context.
    pub fn database_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.database_path = Some(path.into());
        self
    }

    /// Enable/disable network grid downloads (pyproj parity; disabled by default).
    pub fn network_enabled(mut self, enabled: bool) -> Self {
        self.network_enabled = enabled;
        self
    }

    /// Override the user-writable directory used for downloaded grids.
    pub fn user_data_dir(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.user_data_dir = Some(path.into());
        self
    }

    /// Set the CA bundle path used for HTTPS requests.
    pub fn ca_bundle_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.ca_bundle_path = Some(path.into());
        self
    }

    /// Add a data search path.
    pub fn push_data_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.data_paths.push(path.into());
        self
    }
}

/// Internal, serializable options shared by the builder and definition.
///
/// This struct is not part of the public API; use the builder methods.
#[derive(Clone, Debug, Default)]
pub(crate) struct TransformerOptions {
    pub(crate) always_xy: bool,
    pub(crate) area_of_interest: Option<AreaOfInterest>,
    pub(crate) authority: Option<String>,
    pub(crate) desired_accuracy: Option<f64>,
    pub(crate) allow_ballpark: Option<bool>,
    pub(crate) grid_policy: GridPolicy,
}

/// PROJ WKT output version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WktVersion {
    /// WKT2:2019.
    Wkt2_2019,
    /// WKT2:2019, simplified (recommended for interchange).
    Wkt2_2019Simplified,
    /// WKT2:2015.
    Wkt2_2015,
    /// WKT2:2015, simplified.
    Wkt2_2015Simplified,
    /// WKT1 (ESRI flavour), suitable for `.prj` files.
    Wkt1Esri,
    /// WKT1 (GDAL flavour).
    Wkt1Gdal,
}

/// WKT output formatting options passed to `proj_as_wkt`.
///
/// All fields are optional; `None` leaves PROJ's default behaviour. The
/// option strings are the ones documented by PROJ (`MULTILINE=YES`,
/// `INDENTATION_WIDTH`, `ALLOW_ELLIPSOIDAL_HEIGHT_AS_VERTICAL_CRS`,
/// `OUTPUT_AXIS`, `OUTPUT_CONVERSION`, `USE_ALWAYS_XY`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WktOptions {
    /// `MULTILINE=YES` / `NO`.
    pub multiline: Option<bool>,
    /// `INDENTATION_WIDTH` (only meaningful when multiline is enabled).
    pub indentation_width: Option<u32>,
    /// `ALLOW_ELLIPSOIDAL_HEIGHT_AS_VERTICAL_CRS` — accept a
    /// geographic-3D CRS as a bound ("hub") to build a compound vertical CRS.
    pub allow_ellipsoidal_height_as_vertical_crs: Option<bool>,
    /// `OUTPUT_AXIS` — one of `"traditional"`, `"authority"`, `"order"`.
    pub output_axis_order: Option<AxisOutputOrder>,
    /// `OUTPUT_CONVERSION` — whether to output the conversion for projected CRSs.
    pub output_conversion: Option<bool>,
    /// `USE_ALWAYS_XY` — force axis order x/y (east/north).
    pub use_always_xy: Option<bool>,
}

/// `OUTPUT_AXIS` policy for WKT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisOutputOrder {
    /// Output the axes in their traditional (natural) order.
    Traditional,
    /// Output axes in the order mandated by the authority.
    Authority,
    /// Output axes as they appear in the coordinate system ("order").
    Order,
}

/// PROJ-string output format passed to `proj_as_proj_string`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjStringVersion {
    /// WKT2-era PROJ string (`+proj=...`).
    Proj5,
    /// Legacy proj.4 string.
    Proj4,
}
