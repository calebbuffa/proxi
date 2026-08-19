//! Transformer construction and coordinate transformation.

use crate::context::Context;
use crate::coord::{Coord, Coord2, Coord3, Coord4, CoordBatch};
use crate::crs::Crs;
use crate::errors::{ProxiError, Result};
use crate::ffi;
pub use crate::options::{AngularUnits, AreaOfInterest, Direction};
use crate::options::{AreaOfUse, ContextOptions, GridPolicy, TransformerOptions, WktVersion};
use crate::scratch::Scratch;
use std::marker::PhantomData;
use std::path::PathBuf;

/// Resolve the PROJ data directory for a new context, validating that the
/// chosen directory actually contains `proj.db`.
///
/// Priority, per the runtime-data contract:
///   1. An explicit `TransformerOptions.data_dir`.
///   2. The `PROJ_DATA` environment variable.
///   3. The bundled data dir recorded at build time (`PROXI_BUNDLED_DATA_DIR`).
///   4. Nothing — let PROJ use its compiled-in system search paths.
///
/// An *explicitly-provided* `data_dir` that lacks `proj.db` is a hard error
/// (not silently ignored): the user asked for a specific directory and getting
/// a different one silently would be surprising. Environment-derived paths
/// that lack the database are skipped with a warning (the env var may point at
/// a stub or be overridden by the bundled dir).
fn resolve_data_dir(options: &TransformerOptions) -> Result<Option<PathBuf>> {
    let has_db = |dir: &PathBuf| -> bool { dir.join("proj.db").exists() };

    if let Some(dir) = &options.data_dir {
        if !has_db(dir) {
            return Err(ProxiError::MissingData {
                message: format!(
                    "data_dir {} does not contain proj.db (set PROJ_DATA/PROXI_BUNDLED_DATA_DIR or provide a valid data_dir)",
                    dir.display()
                ),
            });
        }
        return Ok(Some(dir.clone()));
    }

    let mut chosen: Option<PathBuf> = None;
    if let Ok(dir) = std::env::var("PROJ_DATA") {
        let dir = PathBuf::from(&dir);
        if has_db(&dir) {
            chosen = Some(dir);
        } else {
            eprintln!(
                "PROXI: PROJ_DATA={} does not contain proj.db; ignoring",
                dir.display()
            );
        }
    }
    if chosen.is_none() {
        if let Ok(dir) = std::env::var("PROXI_BUNDLED_DATA_DIR") {
            let dir = PathBuf::from(dir);
            if has_db(&dir) {
                chosen = Some(dir);
            }
        }
    }
    Ok(chosen)
}

/// Configure a full PROJ context from `ContextOptions` (network, user data
/// dir, CA bundle, database path, extra search paths). Called before any CRS
/// or transformer object is created.
pub(crate) fn configure_context(ctx: &Context, options: &TransformerOptions) -> Result<()> {
    // Resolve the selected data dir once (errors on an explicit invalid dir).
    let resolved_data_dir = resolve_data_dir(options)?;

    // 1. Data search paths: explicit data_dir, then ContextOptions.data_paths.
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(dir) = &resolved_data_dir {
        paths.push(dir.clone());
    }
    if let Some(dir) = &options.context.user_data_dir {
        // Ensure the download target directory exists and is on the search
        // path so downloaded grids are discovered.
        let _ = std::fs::create_dir_all(dir);
        paths.push(dir.clone());
    }
    paths.extend(options.context.data_paths.iter().cloned());
    if !paths.is_empty() {
        ctx.set_search_paths(&paths)?;
    }

    // NOTE: The network/ca-bundle APIs below require PROJ to be built
    // with the relevant support. When the default build (no `network`/`tiff`
    // feature) is used, libproj is compiled without CURL/TIFF and those symbols
    // may be absent or behave differently. We only exercise them when the
    // corresponding cargo feature is enabled.

    // 2. Database: always configure the selected proj.db when a data dir
    // resolved, including offline vertical-datum transformations.
    // `Context::set_database_path` is hardened (it validates `proj.db` exists
    // and round-trips the active path), so a misconfigured data dir fails fast
    // here with a clear `MissingData`/`ContextConfiguration` error instead of
    // surfacing later as a confusing "no database" query failure.
    if let Some(dir) = &resolved_data_dir {
        let db = dir.join("proj.db");
        let aux: Vec<std::path::PathBuf> =
            paths.iter().filter(|path| *path != dir).cloned().collect();
        ctx.set_database_path(&db, &aux, &[])?;
    }

    // 3. Network: disabled by default; opt-in via `network_enabled(true)`.
    //    Only meaningful when built with the `network` feature.
    #[cfg(feature = "network")]
    {
        if options.context.network_enabled {
            ctx.set_network_enabled(true);
        }
        let effective = ctx.network_enabled();
        if effective != options.context.network_enabled {
            eprintln!(
                "proxi: requested network_enabled={} but effective is {}",
                options.context.network_enabled, effective
            );
        }
    }

    // 4. User-writable directory (where PROJ stores downloaded grids). This
    //    MUST be added to the context search paths so grids downloaded into it
    //    are found afterward (pyproj does the same). Only relevant when
    //    network/TIFF support is compiled in.
    #[cfg(feature = "network")]
    {
        if let Some(dir) = ctx.user_writable_directory(true) {
            if !paths.contains(&dir) {
                paths.push(dir);
                ctx.set_search_paths(&paths)?;
            }
        }
    }

    // 5. CA bundle for HTTPS grid downloads.
    #[cfg(feature = "network")]
    if let Some(ca) = &options.context.ca_bundle_path {
        ctx.set_ca_bundle_path(ca)?;
    }
    Ok(())
}

pub(crate) fn configure_context_options(ctx: &Context, options: &ContextOptions) -> Result<()> {
    let transformer_options = TransformerOptions {
        context: options.clone(),
        ..TransformerOptions::default()
    };
    configure_context(ctx, &transformer_options)
}

/// A reusable, immutable transformation definition.
///
/// Stores only serialized CRS strings and options — it is `Send + Sync` and
/// can be shared across threads (e.g. cloned into each Rayon job). A worker
/// calls [`TransformerDefinition::build_for_current_thread`] to obtain a
/// thread-bound [`Transformer`].
#[derive(Clone, Debug)]
pub struct TransformerDefinition {
    source: Option<String>,
    target: Option<String>,
    pipeline: Option<String>,
    options: TransformerOptions,
}

/// One parameter of a coordinate operation's method (e.g. "False easting").
#[derive(Clone, Debug, PartialEq)]
pub struct OperationParameter {
    /// Parameter name, e.g. "False easting".
    pub name: Option<String>,
    /// Authority of the parameter EPSG identifier.
    pub authority: Option<String>,
    /// Code of the parameter EPSG identifier.
    pub code: Option<String>,
    /// Numeric value (when the unit is numeric), in the unit given below.
    pub value: Option<f64>,
    /// String value (for non-numeric parameters).
    pub value_string: Option<String>,
    /// Conversion factor of the unit to the canonical unit (e.g. to metre).
    pub unit_conversion_factor: Option<f64>,
    /// Unit name.
    pub unit_name: Option<String>,
    /// Unit authority.
    pub unit_authority: Option<String>,
    /// Unit code.
    pub unit_code: Option<String>,
    /// Unit category (e.g. "linear" / "angular").
    pub unit_category: Option<String>,
}

/// Owned metadata describing the selected PROJ coordinate operation.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationInfo {
    pub id: Option<String>,
    pub description: Option<String>,
    pub definition: Option<String>,
    pub has_inverse: bool,
    /// Accuracy in metres; negative values mean unknown per PROJ.
    pub accuracy: f64,
    pub area_of_use: Option<AreaOfUse>,
    pub source_crs_wkt: Option<String>,
    pub target_crs_wkt: Option<String>,
    pub method_name: Option<String>,
    pub method_authority: Option<String>,
    pub method_code: Option<String>,
    /// Whether the operation is instantiable (computable) per PROJ. None when
    /// the operation type doesn't support the query.
    pub instantiable: Option<bool>,
    pub has_ballpark_transformation: Option<bool>,
    /// Parameters of the operation's method (empty for non-conversion/
    /// non-transformation operations).
    pub parameters: Vec<OperationParameter>,
}

/// The readiness of a grid file referenced by a coordinate operation.
///
/// This replaces an ambiguous boolean with an explicit, structured result so
/// callers can distinguish *why* a grid is unusable (missing vs. downloadable
/// vs. network-disabled vs. unavailable), which is essential for correct
/// offline/online behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridReadiness {
    /// The grid is present on disk and usable.
    Ready,
    /// Required by the operation but absent; it is direct-downloadable, so
    /// `download_missing_grids` (or `GridPolicy::DownloadMissing`) can fetch it.
    Downloadable,
    /// Required but absent and not directly downloadable (e.g. a package that
    /// must be installed another way).
    Unavailable,
    /// Grid downloads are not possible in the current build/config (no
    /// `network` feature, or network disabled at the PROJ level).
    NetworkDisabled,
}

/// A grid referenced by a coordinate operation.
///
/// `readiness` is the authoritative status; `is_available()` is a convenience
/// derived from it (`Ready`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridInfo {
    pub short_name: Option<String>,
    pub full_name: Option<String>,
    pub package_name: Option<String>,
    pub url: Option<String>,
    pub direct_download: bool,
    pub open_license: bool,
    pub readiness: GridReadiness,
}

impl GridInfo {
    /// Whether the grid is usable right now (i.e. `readiness == Ready`).
    pub fn is_available(&self) -> bool {
        self.readiness == GridReadiness::Ready
    }
}

/// A structured report of an operation's grid requirements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridReport {
    /// Per-grid details, in requirement order.
    pub grids: Vec<GridInfo>,
}

impl GridReport {
    /// Whether every required grid is ready.
    pub fn all_ready(&self) -> bool {
        self.grids.iter().all(|g| g.is_available())
    }

    /// The count of grids that are *not* ready.
    pub fn missing_count(&self) -> usize {
        self.grids.iter().filter(|g| !g.is_available()).count()
    }

    /// Names (short/full) of grids that are not ready, for diagnostics.
    pub fn missing_names(&self) -> Vec<String> {
        self.grids
            .iter()
            .filter(|g| !g.is_available())
            .map(|g| {
                g.short_name
                    .as_deref()
                    .or(g.full_name.as_deref())
                    .unwrap_or("unknown grid")
                    .to_string()
            })
            .collect()
    }
}

/// How PROJ uses source/target CRS extents when selecting operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrsExtentUse {
    None,
    Both,
    Intersection,
    Smallest,
}

/// How PROJ applies the area-of-interest spatial criterion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpatialCriterion {
    StrictContainment,
    PartialIntersection,
}

/// How PROJ treats grid availability when selecting operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridAvailabilityUse {
    UsedForSorting,
    DiscardOperationIfMissingGrid,
    Ignored,
    KnownAvailable,
}

/// Builder for a PROJ operation candidate group.
pub struct TransformerGroupBuilder<'context> {
    context: &'context Context,
    source: String,
    target: String,
    authority: Option<String>,
    desired_accuracy: Option<f64>,
    area_of_interest: Option<AreaOfInterest>,
    discard_superseded: bool,
    allow_ballpark: Option<bool>,
    crs_extent_use: Option<CrsExtentUse>,
    spatial_criterion: Option<SpatialCriterion>,
    grid_availability_use: Option<GridAvailabilityUse>,
    use_proj_alternative_grid_names: Option<bool>,
}

/// A ranked set of coordinate operations produced by PROJ's operation factory.
pub struct TransformerGroup<'context> {
    context: &'context Context,
    operations: Vec<ffi::ProjObj>,
}

impl<'context> TransformerGroupBuilder<'context> {
    pub fn new(
        context: &'context Context,
        source: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            context,
            source: source.into(),
            target: target.into(),
            authority: None,
            desired_accuracy: None,
            area_of_interest: None,
            discard_superseded: true,
            allow_ballpark: None,
            crs_extent_use: None,
            spatial_criterion: None,
            grid_availability_use: None,
            use_proj_alternative_grid_names: None,
        }
    }

    pub fn authority(mut self, authority: impl Into<String>) -> Self {
        self.authority = Some(authority.into());
        self
    }

    pub fn desired_accuracy(mut self, accuracy_meters: f64) -> Self {
        self.desired_accuracy = Some(accuracy_meters);
        self
    }

    pub fn area_of_interest(mut self, area: AreaOfInterest) -> Self {
        self.area_of_interest = Some(area);
        self
    }

    pub fn discard_superseded(mut self, discard: bool) -> Self {
        self.discard_superseded = discard;
        self
    }

    pub fn allow_ballpark(mut self, allow: bool) -> Self {
        self.allow_ballpark = Some(allow);
        self
    }

    pub fn crs_extent_use(mut self, extent: CrsExtentUse) -> Self {
        self.crs_extent_use = Some(extent);
        self
    }

    pub fn spatial_criterion(mut self, criterion: SpatialCriterion) -> Self {
        self.spatial_criterion = Some(criterion);
        self
    }

    pub fn grid_availability_use(mut self, avail: GridAvailabilityUse) -> Self {
        self.grid_availability_use = Some(avail);
        self
    }

    pub fn use_proj_alternative_grid_names(mut self, enabled: bool) -> Self {
        self.use_proj_alternative_grid_names = Some(enabled);
        self
    }

    pub fn build(self) -> Result<TransformerGroup<'context>> {
        let area = self.area_of_interest.map(|area| {
            [
                area.west_lon_degree,
                area.south_lat_degree,
                area.east_lon_degree,
                area.north_lat_degree,
            ]
        });
        let operations = ffi::create_operation_candidates(
            self.context,
            &self.source,
            &self.target,
            self.authority.as_deref(),
            self.desired_accuracy,
            area,
            self.discard_superseded,
            self.allow_ballpark,
            self.crs_extent_use,
            self.spatial_criterion,
            self.grid_availability_use,
            self.use_proj_alternative_grid_names,
        )?;
        Ok(TransformerGroup {
            context: self.context,
            operations,
        })
    }
}

impl<'context> TransformerGroup<'context> {
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    pub fn operation_info(&self, index: usize) -> Result<OperationInfo> {
        let operation = self
            .operations
            .get(index)
            .ok_or_else(|| ProxiError::Transform {
                code: 0,
                message: format!("operation index {index} is out of range"),
            })?;
        operation_info_for(operation, self.context)
    }

    pub fn grids(&self, index: usize) -> Result<Vec<GridInfo>> {
        let operation = self
            .operations
            .get(index)
            .ok_or_else(|| ProxiError::Transform {
                code: 0,
                message: format!("operation index {index} is out of range"),
            })?;
        ffi::operation_grids(operation, self.context)
    }

    pub fn grid_report(&self, index: usize) -> Result<GridReport> {
        Ok(GridReport {
            grids: self.grids(index)?,
        })
    }

    /// Consume the group and promote one candidate to a reusable Transformer.
    pub fn into_transformer(mut self, index: usize) -> Result<Transformer<'context>> {
        if index >= self.operations.len() {
            return Err(ProxiError::Transform {
                code: 0,
                message: format!("operation index {index} is out of range"),
            });
        }
        let obj = self.operations.remove(index);
        let context = self.context;
        let units = UnitMetadata::from_operation(&obj);
        Ok(Transformer {
            obj,
            context,
            units,
            scratch: Scratch::new(),
            _not_send_sync: PhantomData,
        })
    }
}

impl TransformerDefinition {
    /// Create a definition from CRS strings (EPSG codes, WKT, PROJ strings).
    ///
    /// The strings are validated when a [`Transformer`] is built.
    pub fn from_crs(source: impl Into<String>, target: impl Into<String>) -> Result<Self> {
        Ok(Self {
            source: Some(source.into()),
            target: Some(target.into()),
            pipeline: None,
            options: TransformerOptions::default(),
        })
    }

    /// Create an immutable definition from an explicit PROJ pipeline.
    pub fn from_pipeline(pipeline: impl Into<String>) -> Self {
        Self {
            source: None,
            target: None,
            pipeline: Some(pipeline.into()),
            options: TransformerOptions::default(),
        }
    }

    /// Normalize axis order to x/y (e.g. lon/lat instead of lat/lon for
    /// geographic CRSs). Mirrors pyproj's `always_xy`.
    pub fn always_xy(mut self, enabled: bool) -> Self {
        self.options.always_xy = enabled;
        self
    }

    /// Restrict the area of interest used when selecting an operation.
    pub fn area_of_interest(mut self, area: AreaOfInterest) -> Self {
        self.options.area_of_interest = Some(area);
        self
    }

    /// Set an explicit PROJ data directory (containing `proj.db`, grids).
    pub fn data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.options.data_dir = Some(path.into());
        self
    }

    /// Apply full [`ContextOptions`] (network, user data dir, CA bundle, paths).
    pub fn context_options(mut self, ctx: ContextOptions) -> Self {
        self.options.context = ctx;
        self
    }

    /// Enable/disable network grid downloads (pyproj parity, off by default).
    pub fn network_enabled(mut self, enabled: bool) -> Self {
        self.options.context.network_enabled = enabled;
        self
    }

    /// Restrict operation selection to an authority such as `EPSG`.
    pub fn authority(mut self, authority: impl Into<String>) -> Self {
        self.options.authority = Some(authority.into());
        self
    }

    /// Restrict selection to operations at or better than this accuracy in metres.
    pub fn desired_accuracy(mut self, accuracy_meters: f64) -> Self {
        self.options.desired_accuracy = Some(accuracy_meters);
        self
    }

    /// Explicitly allow or reject ballpark transformations.
    pub fn allow_ballpark(mut self, allow: bool) -> Self {
        self.options.allow_ballpark = Some(allow);
        self
    }

    pub fn grid_policy(mut self, policy: GridPolicy) -> Self {
        self.options.grid_policy = policy;
        self
    }

    /// The source CRS definition as WKT, materialized on the caller-provided
    /// context (no hidden context created).
    pub fn source_wkt(&self, context: &Context, version: WktVersion) -> Result<String> {
        let source = self
            .source
            .as_deref()
            .ok_or_else(|| ProxiError::InvalidCrs {
                input: "<pipeline>".to_string(),
                message: "pipelines do not have a standalone source CRS".to_string(),
            })?;
        serialize_crs(context, &self.options, source, |crs| crs.to_wkt(version))
    }

    /// The target CRS definition as WKT, materialized on the caller-provided
    /// context (no hidden context created). This is the correct source for
    /// `.prj` output (a CRS, not an operation).
    pub fn target_wkt(&self, context: &Context, version: WktVersion) -> Result<String> {
        let target = self
            .target
            .as_deref()
            .ok_or_else(|| ProxiError::InvalidCrs {
                input: "<pipeline>".to_string(),
                message: "pipelines do not have a standalone target CRS".to_string(),
            })?;
        serialize_crs(context, &self.options, target, |crs| crs.to_wkt(version))
    }

    /// The source CRS definition as PROJJSON, materialized on the caller-
    /// provided context (no hidden context created).
    pub fn source_projjson(&self, context: &Context) -> Result<String> {
        let source = self
            .source
            .as_deref()
            .ok_or_else(|| ProxiError::InvalidCrs {
                input: "<pipeline>".to_string(),
                message: "pipelines do not have a standalone source CRS".to_string(),
            })?;
        serialize_crs(context, &self.options, source, |crs| crs.to_projjson())
    }

    /// The target CRS definition as PROJJSON, materialized on the caller-
    /// provided context (no hidden context created).
    pub fn target_projjson(&self, context: &Context) -> Result<String> {
        let target = self
            .target
            .as_deref()
            .ok_or_else(|| ProxiError::InvalidCrs {
                input: "<pipeline>".to_string(),
                message: "pipelines do not have a standalone target CRS".to_string(),
            })?;
        serialize_crs(context, &self.options, target, |crs| crs.to_projjson())
    }

    /// Build a thread-bound [`Transformer`] on the calling thread.
    pub fn build_for_current_thread<'context>(
        &self,
        context: &'context Context,
    ) -> Result<Transformer<'context>> {
        Transformer::build(
            context,
            self.source.as_deref(),
            self.target.as_deref(),
            self.pipeline.as_deref(),
            &self.options,
        )
    }
}

/// A one-shot builder for a [`Transformer`].
///
/// Consumes itself to produce a thread-bound transformer. Common options are
/// applied via builder methods.
pub struct TransformerBuilder<'context> {
    context: &'context Context,
    source: Option<String>,
    target: Option<String>,
    pipeline: Option<String>,
    options: TransformerOptions,
}

impl<'context> TransformerBuilder<'context> {
    /// Begin building a transformer from CRS strings.
    pub fn new(
        context: &'context Context,
        source: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self {
            context,
            source: Some(source.into()),
            target: Some(target.into()),
            pipeline: None,
            options: TransformerOptions::default(),
        }
    }

    /// Begin building a transformer from an explicit PROJ pipeline.
    pub fn from_pipeline(context: &'context Context, pipeline: impl Into<String>) -> Self {
        Self {
            context,
            source: None,
            target: None,
            pipeline: Some(pipeline.into()),
            options: TransformerOptions::default(),
        }
    }

    /// Normalize axis order to x/y. Mirrors pyproj's `always_xy`.
    pub fn always_xy(mut self, enabled: bool) -> Self {
        self.options.always_xy = enabled;
        self
    }

    /// Restrict the area of interest used when selecting an operation.
    pub fn area_of_interest(mut self, area: AreaOfInterest) -> Self {
        self.options.area_of_interest = Some(area);
        self
    }

    /// Set an explicit PROJ data directory.
    pub fn data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.options.data_dir = Some(path.into());
        self
    }

    /// Apply full [`ContextOptions`] (network, user data dir, CA bundle, paths).
    pub fn context_options(mut self, ctx: ContextOptions) -> Self {
        self.options.context = ctx;
        self
    }

    /// Enable/disable network grid downloads (pyproj parity, off by default).
    pub fn network_enabled(mut self, enabled: bool) -> Self {
        self.options.context.network_enabled = enabled;
        self
    }

    /// Restrict operation selection to an authority such as `EPSG`.
    pub fn authority(mut self, authority: impl Into<String>) -> Self {
        self.options.authority = Some(authority.into());
        self
    }

    /// Restrict selection to operations at or better than this accuracy in metres.
    pub fn desired_accuracy(mut self, accuracy_meters: f64) -> Self {
        self.options.desired_accuracy = Some(accuracy_meters);
        self
    }

    /// Explicitly allow or reject ballpark transformations.
    pub fn allow_ballpark(mut self, allow: bool) -> Self {
        self.options.allow_ballpark = Some(allow);
        self
    }

    pub fn grid_policy(mut self, policy: GridPolicy) -> Self {
        self.options.grid_policy = policy;
        self
    }

    /// Freeze into an immutable, `Send + Sync` reusable definition.
    pub fn into_definition(self) -> TransformerDefinition {
        TransformerDefinition {
            source: self.source,
            target: self.target,
            pipeline: self.pipeline,
            options: self.options,
        }
    }

    /// Build a thread-bound transformer on the calling thread.
    pub fn build(self) -> Result<Transformer<'context>> {
        Transformer::build(
            self.context,
            self.source.as_deref(),
            self.target.as_deref(),
            self.pipeline.as_deref(),
            &self.options,
        )
    }
}

/// A thread-bound, active coordinate operation.
///
/// Owns a PROJ context and operation (`PJ*`); is `!Send` / `!Sync`, so it
/// must be created and used on the same thread. Reuses its internal scratch
/// buffers across calls for zero steady-state allocation.
pub struct Transformer<'context> {
    /// The operation object. Dropped before `context` (field order), so the
    /// context is still alive when `proj_destroy` runs.
    obj: ffi::ProjObj,
    /// Owned context; must outlive `obj`.
    context: &'context Context,
    units: UnitMetadata,
    scratch: Scratch,
    _not_send_sync: PhantomData<std::rc::Rc<()>>,
}

impl<'context> Transformer<'context> {
    fn build(
        context: &'context Context,
        source: Option<&str>,
        target: Option<&str>,
        pipeline: Option<&str>,
        options: &TransformerOptions,
    ) -> Result<Self> {
        configure_context(context, options)?;

        let area = options
            .area_of_interest
            .as_ref()
            .map(|a| {
                ffi::AreaBox::new(
                    a.west_lon_degree,
                    a.south_lat_degree,
                    a.east_lon_degree,
                    a.north_lat_degree,
                )
            })
            .transpose()?;

        let obj = match (source, target, pipeline) {
            (Some(source), Some(target), None) => {
                if options.grid_policy == GridPolicy::AllowMissing {
                    let operation_options = operation_options(options);
                    ffi::create_crs_to_crs(
                        context,
                        source,
                        target,
                        area.as_ref(),
                        &operation_options,
                    )?
                } else {
                    let area_values = options.area_of_interest.map(|area| {
                        [
                            area.west_lon_degree,
                            area.south_lat_degree,
                            area.east_lon_degree,
                            area.north_lat_degree,
                        ]
                    });
                    let mut candidates = ffi::create_operation_candidates(
                        context,
                        source,
                        target,
                        options.authority.as_deref(),
                        options.desired_accuracy,
                        area_values,
                        true,
                        options.allow_ballpark,
                        None,
                        None,
                        None,
                        None,
                    )?;
                    candidates
                        .drain(..)
                        .next()
                        .ok_or_else(|| ProxiError::InvalidTransformer {
                            source_crs: source.to_string(),
                            target_crs: target.to_string(),
                            message: "PROJ returned no usable operation candidates".to_string(),
                        })?
                }
            }
            (None, None, Some(pipeline)) => ffi::create_pipeline(context, pipeline)?,
            _ => {
                return Err(ProxiError::InvalidTransformer {
                    source_crs: source.unwrap_or("<pipeline>").to_string(),
                    target_crs: target.unwrap_or("<pipeline>").to_string(),
                    message: "invalid transformer definition".to_string(),
                });
            }
        };

        // Apply always_xy (normalize for visualization).
        let obj = if options.always_xy {
            ffi::normalize_for_visualization(context, obj)?
        } else {
            obj
        };

        ensure_grid_policy(&obj, context, options)?;

        let units = UnitMetadata::from_operation(&obj);

        Ok(Self {
            obj,
            context,
            units,
            scratch: Scratch::new(),
            _not_send_sync: PhantomData,
        })
    }

    /// Transform a single arbitrary [`Coord`] in-place.
    ///
    /// This is the zero-allocation path for user-provided coordinate types:
    /// it reads `x`/`y` (and `z`/`t` if custom) via the [`Coord`] trait and
    /// writes the result back through [`Coord::from_xyzt`] without boxing.
    pub fn transform_coord<C: Coord>(
        &mut self,
        point: C,
        direction: Direction,
        units: AngularUnits,
    ) -> Result<C> {
        let dir = ffi::dir_code(direction);
        let (in_scale, _) = self.units.scales(dir, units);
        let v = [
            point.x() * in_scale,
            point.y() * in_scale,
            point.z(),
            point.t(),
        ];
        ffi::errno_reset(&self.obj);
        let out = ffi::trans(&self.obj, dir, v);
        check_errno(&self.obj, self.context)?;
        let (_, out_scale) = self.units.scales(dir, units);
        Ok(C::from_xyzt(
            out[0] * out_scale,
            out[1] * out_scale,
            out[2],
            out[3],
        ))
    }

    /// Transform a slice of arbitrary [`Coord`] in place.
    ///
    /// Each element is transformed via [`Coord`] with no intermediate
    /// allocation. This is the scalar path (not `proj_trans_generic`); for
    /// maximum throughput over plain `f64` buffers prefer
    /// [`Transformer::transform_xy_in_place`] (and friends).
    pub fn transform_coords<C: Coord>(
        &mut self,
        coords: &mut [C],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let dir = ffi::dir_code(direction);
        let (in_scale, out_scale) = self.units.scales(dir, units);
        ffi::errno_reset(&self.obj);
        for point in coords.iter_mut() {
            let v = [
                point.x() * in_scale,
                point.y() * in_scale,
                point.z(),
                point.t(),
            ];
            let out = ffi::trans(&self.obj, dir, v);
            *point = C::from_xyzt(out[0] * out_scale, out[1] * out_scale, out[2], out[3]);
        }
        check_errno(&self.obj, self.context)
    }

    /// Transform a 2D point with an explicit direction and angular units.
    pub fn transform_xy(
        &mut self,
        p: Coord2,
        direction: Direction,
        units: AngularUnits,
    ) -> Result<Coord2> {
        let dir = ffi::dir_code(direction);
        let (in_scale, _) = self.units.scales(dir, units);

        let v = [p.x * in_scale, p.y * in_scale, 0.0, f64::INFINITY];
        ffi::errno_reset(&self.obj);
        let out = ffi::trans(&self.obj, dir, v);
        check_errno(&self.obj, self.context)?;

        let (_, out_scale) = self.units.scales(dir, units);
        Ok(Coord2 {
            x: out[0] * out_scale,
            y: out[1] * out_scale,
        })
    }

    /// Transform a 3D point with an explicit direction and angular units.
    pub fn transform_xyz(
        &mut self,
        p: Coord3,
        direction: Direction,
        units: AngularUnits,
    ) -> Result<Coord3> {
        let dir = ffi::dir_code(direction);
        let (in_scale, _) = self.units.scales(dir, units);

        let v = [p.x * in_scale, p.y * in_scale, p.z, f64::INFINITY];
        ffi::errno_reset(&self.obj);
        let out = ffi::trans(&self.obj, dir, v);
        check_errno(&self.obj, self.context)?;

        let (_, out_scale) = self.units.scales(dir, units);
        Ok(Coord3 {
            x: out[0] * out_scale,
            y: out[1] * out_scale,
            z: out[2],
        })
    }

    /// Transform a 4D point with an explicit direction and angular units.
    pub fn transform_xyzt(
        &mut self,
        p: Coord4,
        direction: Direction,
        units: AngularUnits,
    ) -> Result<Coord4> {
        let dir = ffi::dir_code(direction);
        let (in_scale, _) = self.units.scales(dir, units);

        let v = [p.x * in_scale, p.y * in_scale, p.z, p.t];
        ffi::errno_reset(&self.obj);
        let out = ffi::trans(&self.obj, dir, v);
        check_errno(&self.obj, self.context)?;

        let (_, out_scale) = self.units.scales(dir, units);
        Ok(Coord4 {
            x: out[0] * out_scale,
            y: out[1] * out_scale,
            z: out[2],
            t: out[3],
        })
    }

    /// Forward 2D transform (source -> target).
    pub fn forward_xy(&mut self, p: Coord2) -> Result<Coord2> {
        self.transform_xy(p, Direction::Forward, AngularUnits::Auto)
    }

    /// Inverse 2D transform (target -> source).
    pub fn inverse_xy(&mut self, p: Coord2) -> Result<Coord2> {
        self.transform_xy(p, Direction::Inverse, AngularUnits::Auto)
    }

    /// Forward 3D transform (source -> target).
    pub fn forward_xyz(&mut self, p: Coord3) -> Result<Coord3> {
        self.transform_xyz(p, Direction::Forward, AngularUnits::Auto)
    }

    /// Inverse 3D transform (target -> source).
    pub fn inverse_xyz(&mut self, p: Coord3) -> Result<Coord3> {
        self.transform_xyz(p, Direction::Inverse, AngularUnits::Auto)
    }

    /// Forward 4D transform (source -> target).
    pub fn forward_xyzt(&mut self, p: Coord4) -> Result<Coord4> {
        self.transform_xyzt(p, Direction::Forward, AngularUnits::Auto)
    }

    /// Inverse 4D transform (target -> source).
    pub fn inverse_xyzt(&mut self, p: Coord4) -> Result<Coord4> {
        self.transform_xyzt(p, Direction::Inverse, AngularUnits::Auto)
    }

    /// Return metadata for the selected coordinate operation.
    pub fn operation_info(&self) -> Result<OperationInfo> {
        operation_info_for(&self.obj, self.context)
    }

    /// Return grid files used by this operation and their readiness.
    pub fn grids(&self) -> Result<Vec<GridInfo>> {
        ffi::operation_grids(&self.obj, self.context)
    }

    /// Return a structured report of this operation's grid requirements.
    pub fn grid_report(&self) -> Result<GridReport> {
        Ok(GridReport {
            grids: self.grids()?,
        })
    }

    /// Download missing directly-downloadable grids through PROJ's configured
    /// network/cache path. Returns the number of grid files requested.
    pub fn download_missing_grids(&self) -> Result<usize> {
        #[cfg(feature = "network")]
        {
            let grids = self.grids()?;
            let mut downloaded = 0;
            for grid in grids {
                if !grid.is_available() && grid.direct_download {
                    if let Some(url) = grid.url {
                        self.context.download_grid(&url)?;
                        downloaded += 1;
                    }
                }
            }
            Ok(downloaded)
        }
        #[cfg(not(feature = "network"))]
        {
            Err(ProxiError::Unsupported {
                feature: "grid downloads require the network feature",
            })
        }
    }

    /// Transform an axis-aligned 2D bounds rectangle.
    ///
    /// `bounds` is `[west, south, east, north]`. `densify_points` controls
    /// how many intermediate points PROJ samples along each edge. Angular
    /// coordinates use the requested `units`; linear coordinates are unchanged.
    pub fn transform_bounds(
        &mut self,
        bounds: [f64; 4],
        direction: Direction,
        units: AngularUnits,
        densify_points: i32,
    ) -> Result<[f64; 4]> {
        if densify_points < 0 {
            return Err(ProxiError::Transform {
                code: 0,
                message: "densify_points must be non-negative".to_string(),
            });
        }
        let dir = ffi::dir_code(direction);
        let (in_scale, out_scale) = self.units.scales(dir, units);
        let input = [
            bounds[0] * in_scale,
            bounds[1] * in_scale,
            bounds[2] * in_scale,
            bounds[3] * in_scale,
        ];
        let mut output = ffi::trans_bounds(&self.obj, self.context, dir, input, densify_points)?;
        if out_scale != 1.0 {
            for value in &mut output {
                *value *= out_scale;
            }
        }
        Ok(output)
    }

    /// Forward bounds transform using automatic angular-unit handling.
    pub fn forward_bounds(&mut self, bounds: [f64; 4], densify_points: i32) -> Result<[f64; 4]> {
        self.transform_bounds(
            bounds,
            Direction::Forward,
            AngularUnits::Auto,
            densify_points,
        )
    }

    /// Inverse bounds transform using automatic angular-unit handling.
    pub fn inverse_bounds(&mut self, bounds: [f64; 4], densify_points: i32) -> Result<[f64; 4]> {
        self.transform_bounds(
            bounds,
            Direction::Inverse,
            AngularUnits::Auto,
            densify_points,
        )
    }

    /// Transform a validated structure-of-arrays batch in place.
    ///
    /// This is the zero-copy fast path, mapping directly to
    /// `proj_trans_generic`. The `x`/`y` buffers are converted degrees <-> radians
    /// in place as needed, then transformed.
    pub fn transform_soa(
        &mut self,
        coords: CoordBatch<'_>,
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let (x, y, z, t) = coords.into_parts();
        // Defensive check at the FFI boundary: `CoordBatch`'s validating
        // constructors enforce equal lengths, but re-verify here so the
        // safety argument is local to the one function that hands slices to
        // PROJ's `proj_trans_generic` (protects against future internal
        // changes). Only a few length comparisons — negligible vs. a batch.
        if y.len() != x.len()
            || z.as_ref().is_some_and(|z| z.len() != x.len())
            || t.as_ref().is_some_and(|t| t.len() != x.len())
        {
            return Err(ProxiError::LengthMismatch {
                name: "coordinate batch",
                expected: x.len(),
                actual: y.len(),
            });
        }
        // obj and scratch are disjoint fields, so we can borrow them
        // independently without a `&mut self` method call.
        transform_slices_soa(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            x,
            y,
            z,
            t,
        )
    }

    /// In-place 2D batch via separate `x` and `y` slices.
    pub fn transform_xy_in_place(
        &mut self,
        x: &mut [f64],
        y: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let coords = CoordBatch::new(x, y)?;
        self.transform_soa(coords, direction, units)
    }

    pub fn transform_xy_into(
        &mut self,
        input_x: &[f64],
        input_y: &[f64],
        output_x: &mut [f64],
        output_y: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = input_x.len();
        for (name, actual) in [
            ("input_y", input_y.len()),
            ("output_x", output_x.len()),
            ("output_y", output_y.len()),
        ] {
            if actual != n {
                return Err(ProxiError::LengthMismatch {
                    name,
                    expected: n,
                    actual,
                });
            }
        }
        output_x.copy_from_slice(input_x);
        output_y.copy_from_slice(input_y);
        self.transform_xy_in_place(output_x, output_y, direction, units)
    }

    pub fn transform_xy_transactional(
        &mut self,
        x: &mut [f64],
        y: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = x.len();
        if y.len() != n {
            return Err(ProxiError::LengthMismatch {
                name: "y",
                expected: n,
                actual: y.len(),
            });
        }
        if n == 0 {
            return Ok(());
        }
        self.scratch.ensure_capacity(n);
        self.scratch.xs[..n].copy_from_slice(x);
        self.scratch.ys[..n].copy_from_slice(y);
        transform_slices_soa(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            &mut self.scratch.xs[..n],
            &mut self.scratch.ys[..n],
            None,
            None,
        )?;
        x.copy_from_slice(&self.scratch.xs[..n]);
        y.copy_from_slice(&self.scratch.ys[..n]);
        Ok(())
    }

    /// In-place 3D batch via separate `x`, `y`, `z` slices.
    pub fn transform_xyz_in_place(
        &mut self,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let coords = CoordBatch::new(x, y)?.with_z(z)?;
        self.transform_soa(coords, direction, units)
    }

    /// Transform separate XYZ input slices into separate caller-owned output
    /// slices. Inputs are never modified; no internal coordinate re-layout is
    /// required beyond the caller's output buffers.
    #[allow(clippy::too_many_arguments)]
    pub fn transform_xyz_into(
        &mut self,
        input_x: &[f64],
        input_y: &[f64],
        input_z: &[f64],
        output_x: &mut [f64],
        output_y: &mut [f64],
        output_z: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = input_x.len();
        for (name, actual) in [
            ("input_y", input_y.len()),
            ("input_z", input_z.len()),
            ("output_x", output_x.len()),
            ("output_y", output_y.len()),
            ("output_z", output_z.len()),
        ] {
            if actual != n {
                return Err(ProxiError::LengthMismatch {
                    name,
                    expected: n,
                    actual,
                });
            }
        }
        output_x.copy_from_slice(input_x);
        output_y.copy_from_slice(input_y);
        output_z.copy_from_slice(input_z);
        self.transform_xyz_in_place(output_x, output_y, output_z, direction, units)
    }

    /// Transform XYZ data atomically. The input slices are unchanged if PROJ
    /// reports an error; successful results are committed in place.
    pub fn transform_xyz_transactional(
        &mut self,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = x.len();
        if y.len() != n {
            return Err(ProxiError::LengthMismatch {
                name: "y",
                expected: n,
                actual: y.len(),
            });
        }
        if z.len() != n {
            return Err(ProxiError::LengthMismatch {
                name: "z",
                expected: n,
                actual: z.len(),
            });
        }
        if n == 0 {
            return Ok(());
        }
        self.scratch.ensure_capacity(n);
        self.scratch.xs[..n].copy_from_slice(x);
        self.scratch.ys[..n].copy_from_slice(y);
        self.scratch.zs[..n].copy_from_slice(z);
        transform_slices_soa(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            &mut self.scratch.xs[..n],
            &mut self.scratch.ys[..n],
            Some(&mut self.scratch.zs[..n]),
            None,
        )?;
        x.copy_from_slice(&self.scratch.xs[..n]);
        y.copy_from_slice(&self.scratch.ys[..n]);
        z.copy_from_slice(&self.scratch.zs[..n]);
        Ok(())
    }

    /// In-place 4D batch via separate `x`, `y`, `z`, `t` slices.
    pub fn transform_xyzt_in_place(
        &mut self,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        t: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let coords = CoordBatch::new(x, y)?.with_z(z)?.with_t(t)?;
        self.transform_soa(coords, direction, units)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn transform_xyzt_into(
        &mut self,
        input_x: &[f64],
        input_y: &[f64],
        input_z: &[f64],
        input_t: &[f64],
        output_x: &mut [f64],
        output_y: &mut [f64],
        output_z: &mut [f64],
        output_t: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = input_x.len();
        for (name, actual) in [
            ("input_y", input_y.len()),
            ("input_z", input_z.len()),
            ("input_t", input_t.len()),
            ("output_x", output_x.len()),
            ("output_y", output_y.len()),
            ("output_z", output_z.len()),
            ("output_t", output_t.len()),
        ] {
            if actual != n {
                return Err(ProxiError::LengthMismatch {
                    name,
                    expected: n,
                    actual,
                });
            }
        }
        output_x.copy_from_slice(input_x);
        output_y.copy_from_slice(input_y);
        output_z.copy_from_slice(input_z);
        output_t.copy_from_slice(input_t);
        self.transform_xyzt_in_place(output_x, output_y, output_z, output_t, direction, units)
    }

    pub fn transform_xyzt_transactional(
        &mut self,
        x: &mut [f64],
        y: &mut [f64],
        z: &mut [f64],
        t: &mut [f64],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = x.len();
        for (name, actual) in [("y", y.len()), ("z", z.len()), ("t", t.len())] {
            if actual != n {
                return Err(ProxiError::LengthMismatch {
                    name,
                    expected: n,
                    actual,
                });
            }
        }
        if n == 0 {
            return Ok(());
        }
        self.scratch.ensure_capacity(n);
        self.scratch.xs[..n].copy_from_slice(x);
        self.scratch.ys[..n].copy_from_slice(y);
        self.scratch.zs[..n].copy_from_slice(z);
        self.scratch.ts[..n].copy_from_slice(t);
        transform_slices_soa(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            &mut self.scratch.xs[..n],
            &mut self.scratch.ys[..n],
            Some(&mut self.scratch.zs[..n]),
            Some(&mut self.scratch.ts[..n]),
        )?;
        x.copy_from_slice(&self.scratch.xs[..n]);
        y.copy_from_slice(&self.scratch.ys[..n]);
        z.copy_from_slice(&self.scratch.zs[..n]);
        t.copy_from_slice(&self.scratch.ts[..n]);
        Ok(())
    }

    /// Transform an array of `[x, y, z]` in place.
    ///
    /// Stages through a reusable internal scratch buffer (allocates on first
    /// growth, then copies twice): no steady-state allocation after warmup.
    pub fn transform_xyz_aos_in_place(
        &mut self,
        coords: &mut [[f64; 3]],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = coords.len();
        if n == 0 {
            return Ok(());
        }
        self.scratch.ensure_capacity(n);
        for (i, c) in coords.iter().enumerate() {
            self.scratch.xs[i] = c[0];
            self.scratch.ys[i] = c[1];
            self.scratch.zs[i] = c[2];
        }

        // Borrow obj/context and scratch as disjoint fields.
        transform_slices_soa(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            &mut self.scratch.xs[..n],
            &mut self.scratch.ys[..n],
            Some(&mut self.scratch.zs[..n]),
            None,
        )?;

        for (i, c) in coords.iter_mut().enumerate() {
            c[0] = self.scratch.xs[i];
            c[1] = self.scratch.ys[i];
            c[2] = self.scratch.zs[i];
        }
        Ok(())
    }

    /// Transform an array of `[x, y, z]` in place, returning the number of
    /// points processed (must equal `len()` on success). Used by the
    /// completeness (`NaN/Inf`) policy path. Internal.
    fn transform_xyz_aos_in_place_partial(
        &mut self,
        coords: &mut [[f64; 3]],
        direction: Direction,
        units: AngularUnits,
    ) -> std::result::Result<usize, crate::errors::PartialFailure> {
        let n = coords.len();
        if n == 0 {
            return Ok(0);
        }
        self.scratch.ensure_capacity(n);
        for (i, c) in coords.iter().enumerate() {
            self.scratch.xs[i] = c[0];
            self.scratch.ys[i] = c[1];
            self.scratch.zs[i] = c[2];
        }
        let processed = transform_slices_soa_partial(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            &mut self.scratch.xs[..n],
            &mut self.scratch.ys[..n],
            Some(&mut self.scratch.zs[..n]),
            None,
        )?;
        for (i, c) in coords.iter_mut().enumerate() {
            c[0] = self.scratch.xs[i];
            c[1] = self.scratch.ys[i];
            c[2] = self.scratch.zs[i];
        }
        Ok(processed)
    }

    /// Transform an array of `[x, y, z, t]` in place.
    ///
    /// Stages through a reusable internal scratch buffer; no steady-state
    /// allocation after warmup.
    pub fn transform_xyzt_aos_in_place(
        &mut self,
        coords: &mut [[f64; 4]],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<()> {
        let n = coords.len();
        if n == 0 {
            return Ok(());
        }
        self.scratch.ensure_capacity(n);
        for (i, c) in coords.iter().enumerate() {
            self.scratch.xs[i] = c[0];
            self.scratch.ys[i] = c[1];
            self.scratch.zs[i] = c[2];
            self.scratch.ts[i] = c[3];
        }

        transform_slices_soa(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            &mut self.scratch.xs[..n],
            &mut self.scratch.ys[..n],
            Some(&mut self.scratch.zs[..n]),
            Some(&mut self.scratch.ts[..n]),
        )?;

        for (i, c) in coords.iter_mut().enumerate() {
            c[0] = self.scratch.xs[i];
            c[1] = self.scratch.ys[i];
            c[2] = self.scratch.zs[i];
            c[3] = self.scratch.ts[i];
        }
        Ok(())
    }

    /// Transform an interleaved array of `[x, y]` points in place using the
    /// transformer's reusable scratch buffers.
    ///
    /// This is the AoS (array-of-structures) companion to the SOA fast path;
    /// it copies into separate scratch buffers and copies results back.
    /// On a *short* PROJ count it returns `Err(PartialFailure)`
    /// with the committed-prefix summary (PROJ committed the first `processed`
    /// points in place; `total` is the batch size). Non-finite inputs are
    /// included in the count (see NaN/Inf policy).
    pub fn transform_xy_interleaved_in_place(
        &mut self,
        coords: &mut [[f64; 2]],
        direction: Direction,
        units: AngularUnits,
    ) -> std::result::Result<usize, crate::errors::PartialFailure> {
        let n = coords.len();
        if n == 0 {
            return Ok(0);
        }
        self.scratch.ensure_capacity(n);
        for (i, c) in coords.iter().enumerate() {
            self.scratch.xs[i] = c[0];
            self.scratch.ys[i] = c[1];
        }
        let processed = transform_slices_soa_partial(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            &mut self.scratch.xs[..n],
            &mut self.scratch.ys[..n],
            None,
            None,
        )?;
        for (i, c) in coords.iter_mut().enumerate() {
            c[0] = self.scratch.xs[i];
            c[1] = self.scratch.ys[i];
        }
        Ok(processed)
    }

    /// Transform `[x, y, z]` points in place with a completeness (NaN/Inf)
    /// policy, reporting any partial failure via [`PartialFailure`].
    ///
    /// NaN/Inf policy: a point whose `x` or `y` is non-finite is *not* sent to
    /// PROJ. It counts toward `total` but not `processed`, and its output is
    /// left unchanged. `z` is carried through even when it is non-finite
    /// (height is typically the ellipsoid/vertical component, not the angular
    /// driver). The first non-finite point truncates the processed prefix, so
    /// `processed <= first_bad_point` unless every input is finite.
    pub fn transform_xyz_complete(
        &mut self,
        coords: &mut [[f64; 3]],
        direction: Direction,
        units: AngularUnits,
    ) -> std::result::Result<usize, crate::errors::PartialFailure> {
        let total = coords.len();
        // Find the first non-finite (x|y) point; points before it are all finite.
        let first_bad = coords
            .iter()
            .position(|c| !c[0].is_finite() || !c[1].is_finite())
            .unwrap_or(total);
        // Transform the all-finite prefix in place (zero-copy).
        let prefix = &mut coords[..first_bad];
        let processed = self.transform_xyz_aos_in_place_partial(prefix, direction, units)?;
        debug_assert_eq!(processed, first_bad);
        if processed == total {
            Ok(processed)
        } else {
            Err(crate::errors::PartialFailure { processed, total })
        }
    }
}

fn operation_info_for(obj: &ffi::ProjObj, context: &Context) -> Result<OperationInfo> {
    let info = ffi::pj_info(obj);
    let area = ffi::area_of_use(obj, context)?.map(|(west, south, east, north, name)| AreaOfUse {
        west_lon_degree: west,
        south_lat_degree: south,
        east_lon_degree: east,
        north_lat_degree: north,
        name,
    });
    let (source_crs_wkt, target_crs_wkt) = ffi::operation_crs_wkts(obj, context);
    let (method_name, method_authority, method_code, has_ballpark_transformation) =
        ffi::operation_method(obj, context);
    let instantiable = ffi::operation_instantiable(obj, context);
    let parameters = ffi::operation_parameters(obj, context);
    Ok(OperationInfo {
        id: ffi::cstr_opt(info.id),
        description: ffi::cstr_opt(info.description),
        definition: ffi::cstr_opt(info.definition),
        has_inverse: info.has_inverse != 0,
        accuracy: info.accuracy,
        area_of_use: area,
        source_crs_wkt,
        target_crs_wkt,
        method_name,
        method_authority,
        method_code,
        instantiable,
        has_ballpark_transformation,
        parameters,
    })
}

/// Whether the given PROJ operation, under the configured network policy, can
/// download the missing grid it requires.
fn network_can_download(_options: &TransformerOptions) -> bool {
    // The `network` cargo feature must be enabled, and the context must have
    // network enabled (handled by the caller checking `options.context`).
    #[cfg(feature = "network")]
    {
        true
    }
    #[cfg(not(feature = "network"))]
    {
        false
    }
}

fn ensure_grid_policy(
    obj: &ffi::ProjObj,
    context: &Context,
    options: &TransformerOptions,
) -> Result<()> {
    if options.grid_policy == GridPolicy::AllowMissing {
        return Ok(());
    }
    let mut grids = ffi::operation_grids(obj, context)?;
    let missing = || grids.iter().filter(|grid| !grid.is_available()).count();
    if missing() == 0 {
        return Ok(());
    }
    if options.grid_policy == GridPolicy::DownloadMissing
        && network_can_download(options)
        && options.context.network_enabled
    {
        for grid in grids
            .iter()
            .filter(|grid| !grid.is_available() && grid.direct_download)
        {
            if let Some(url) = &grid.url {
                context.download_grid(url)?;
            }
        }
        grids = ffi::operation_grids(obj, context)?;
        if grids.iter().all(|grid| grid.is_available()) {
            return Ok(());
        }
    }
    let missing_names = grids
        .iter()
        .filter(|grid| !grid.is_available())
        .map(|grid| {
            grid.short_name
                .as_deref()
                .or(grid.full_name.as_deref())
                .unwrap_or("unknown grid")
        })
        .collect::<Vec<_>>();
    Err(ProxiError::GridMissing {
        message: format!("missing grid(s): {}", missing_names.join(", ")),
    })
}

/// Serialize a CRS user-input string via an explicit, caller-owned context.
///
/// This takes a `&Context` (no hidden context creation): the caller is
/// responsible for providing/owning a configured context.
fn serialize_crs<'context, T>(
    context: &'context Context,
    options: &TransformerOptions,
    input: &str,
    serialize: impl FnOnce(&Crs<'context>) -> Result<T>,
) -> Result<T> {
    configure_context(context, options)?;
    let crs = Crs::from_user_input(context, input)?;
    serialize(&crs)
}

fn operation_options(options: &TransformerOptions) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(authority) = &options.authority {
        values.push(format!("AUTHORITY={authority}"));
    }
    if let Some(accuracy) = options.desired_accuracy {
        values.push(format!("ACCURACY={accuracy}"));
    }
    if let Some(allow) = options.allow_ballpark {
        values.push(format!(
            "ALLOW_BALLPARK={}",
            if allow { "YES" } else { "NO" }
        ));
    }
    values
}

#[derive(Clone, Copy)]
struct UnitInfo {
    angular_input: bool,
    angular_output: bool,
    degree_input: bool,
    degree_output: bool,
}

#[derive(Clone, Copy)]
struct UnitMetadata {
    forward: UnitInfo,
    inverse: UnitInfo,
}

impl UnitMetadata {
    fn from_operation(obj: &ffi::ProjObj) -> Self {
        let forward = Self::read(obj, crate::bindings::PJ_FWD);
        let inverse = Self::read(obj, crate::bindings::PJ_INV);
        Self { forward, inverse }
    }

    fn read(obj: &ffi::ProjObj, dir: crate::bindings::PJ_DIRECTION) -> UnitInfo {
        UnitInfo {
            angular_input: ffi::angular_input(obj, dir),
            angular_output: ffi::angular_output(obj, dir),
            degree_input: ffi::degree_input(obj, dir),
            degree_output: ffi::degree_output(obj, dir),
        }
    }

    fn scales(&self, dir: crate::bindings::PJ_DIRECTION, units: AngularUnits) -> (f64, f64) {
        let info = if dir == crate::bindings::PJ_INV {
            self.inverse
        } else {
            self.forward
        };
        match units {
            // `Auto` matches PROJ's native convention: the caller supplies
            // degrees for angular operations (PROJ works internally in
            // radians, converting on input) and receives degrees on output.
            // For linear (projected) operations that don't report angular or
            // degree input, no conversion is applied. This is the only mode
            // that genuinely inspects the operation; `Degrees`/`Radians` are
            // explicit user overrides.
            AngularUnits::Auto => {
                let in_scale = if info.angular_input && !info.degree_input {
                    ffi::DG2RAD // op expects radians, caller supplied degrees
                } else {
                    1.0 // op uses degrees natively, or is linear
                };
                let out_scale = if info.angular_output && !info.degree_output {
                    ffi::RAD2DG // op produced radians, caller wants degrees
                } else {
                    1.0 // op produced degrees, or is linear
                };
                (in_scale, out_scale)
            }
            // User forces degree interpretation: convert to radians on input
            // for angular ops, back on output.
            AngularUnits::Degrees => {
                let in_scale = if info.angular_input { ffi::DG2RAD } else { 1.0 };
                let out_scale = if info.angular_output {
                    ffi::RAD2DG
                } else {
                    1.0
                };
                (in_scale, out_scale)
            }
            // User forces radian interpretation: convert from radians on input
            // if the op expects degrees, and to radians on output.
            AngularUnits::Radians => {
                let in_scale = if info.degree_input { ffi::RAD2DG } else { 1.0 };
                let out_scale = if info.degree_output { ffi::DG2RAD } else { 1.0 };
                (in_scale, out_scale)
            }
        }
    }
}

/// Transform unit scales cached from the operation's input/output metadata.
fn input_output_scales(
    units_metadata: &UnitMetadata,
    dir: crate::bindings::PJ_DIRECTION,
    units: AngularUnits,
) -> (f64, f64) {
    units_metadata.scales(dir, units)
}

/// Check PROJ error state after a transform; return an error if set.
fn check_errno(obj: &ffi::ProjObj, context: &Context) -> Result<()> {
    let code = ffi::errno(obj);
    if code != 0 {
        let message = ffi::errno_string(context, code);
        Err(ProxiError::Transform { code, message })
    } else {
        Ok(())
    }
}

/// Core SOA transform: apply unit scaling, call `proj_trans_generic`, then
/// reverse scaling. All slices must be the same length (validated by
/// [`CoordBatch`] or by the caller's invariants).
#[allow(clippy::too_many_arguments)]
fn transform_slices_soa(
    obj: &ffi::ProjObj,
    context: &Context,
    direction: Direction,
    units: AngularUnits,
    units_metadata: &UnitMetadata,
    x: &mut [f64],
    y: &mut [f64],
    z: Option<&mut [f64]>,
    t: Option<&mut [f64]>,
) -> Result<()> {
    let dir = ffi::dir_code(direction);
    let (in_scale, out_scale) = input_output_scales(units_metadata, dir, units);

    let n = x.len();
    if n == 0 {
        return Ok(());
    }

    // Input unit scaling on x/y only (z and t are not angular).
    if in_scale != 1.0 {
        for v in x.iter_mut() {
            *v *= in_scale;
        }
        for v in y.iter_mut() {
            *v *= in_scale;
        }
    }

    ffi::errno_reset(obj);
    let n_out = ffi::trans_generic(obj, dir, x, y, z, t);
    check_errno(obj, context)?;
    // `proj_trans_generic` returns the number of coordinates successfully
    // transformed; a short count indicates a partial failure.
    if n_out != n {
        return Err(ProxiError::Transform {
            code: ffi::errno(obj),
            message: format!(
                "only {n_out} of {n} coordinates were transformed (PROJ stopped early)"
            ),
        });
    }

    // Output unit scaling on x/y only.
    if out_scale != 1.0 {
        for v in x.iter_mut() {
            *v *= out_scale;
        }
        for v in y.iter_mut() {
            *v *= out_scale;
        }
    }

    Ok(())
}

/// Core SOA transform that reports a short PROJ count as a partial failure
/// instead of a hard error (M5).
///
/// On success returns `Ok(processed)` where `processed == n`. If PROJ stops
/// early, returns `Err(PartialFailure{processed, total})`. `proj_trans_generic`
/// already committed the first `processed` coordinates in place; callers that
/// need atomicity must use the transactional wrappers.
#[allow(clippy::too_many_arguments)]
fn transform_slices_soa_partial(
    obj: &ffi::ProjObj,
    _context: &Context,
    direction: Direction,
    units: AngularUnits,
    units_metadata: &UnitMetadata,
    x: &mut [f64],
    y: &mut [f64],
    z: Option<&mut [f64]>,
    t: Option<&mut [f64]>,
) -> std::result::Result<usize, crate::errors::PartialFailure> {
    let n = x.len();
    // Input unit scaling on x/y only.
    let dir = ffi::dir_code(direction);
    let (in_scale, out_scale) = input_output_scales(units_metadata, dir, units);
    if in_scale != 1.0 {
        for v in x.iter_mut() {
            *v *= in_scale;
        }
        for v in y.iter_mut() {
            *v *= in_scale;
        }
    }
    ffi::errno_reset(obj);
    let n_out = ffi::trans_generic(obj, dir, x, y, z, t);
    // PROJ may commit only a prefix. The input unit conversion above applies
    // to the whole slice, so restore the untouched suffix before returning.
    if n_out < n && in_scale != 1.0 {
        for value in &mut x[n_out..] {
            *value /= in_scale;
        }
        for value in &mut y[n_out..] {
            *value /= in_scale;
        }
    }
    // Output unit scaling on the successfully-transformed prefix only.
    if out_scale != 1.0 {
        for v in x[..n_out].iter_mut() {
            *v *= out_scale;
        }
        for v in y[..n_out].iter_mut() {
            *v *= out_scale;
        }
    }
    if n_out == n {
        Ok(n)
    } else {
        Err(crate::errors::PartialFailure {
            processed: n_out,
            total: n,
        })
    }
}
