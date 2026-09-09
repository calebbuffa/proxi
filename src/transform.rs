//! Transformer construction and coordinate transformation.

use crate::context::Context;
use crate::coord::{Coord, CoordBatch};
use crate::crs::Crs;
use crate::errors::{ProxiError, Result};
use crate::ffi;
pub use crate::options::{AngularUnits, AreaOfInterest, Direction};
use crate::options::{AreaOfUse, GridPolicy, TransformerOptions, WktVersion};
use crate::scratch::Scratch;
use std::marker::PhantomData;

/// A reusable, immutable transformation definition.
///
/// Stores only serialized CRS strings and options - it is `Send + Sync` and
/// can be shared across threads (e.g. cloned into each Rayon job). A worker
/// calls [`TransformerDefinition::build`] to obtain a thread-bound
/// [`Transformer`] from that thread's own [`Context`].
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
        serialize_crs(context, source, |crs| crs.to_wkt(version, None))
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
        serialize_crs(context, target, |crs| crs.to_wkt(version, None))
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
        serialize_crs(context, source, |crs| crs.to_projjson())
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
        serialize_crs(context, target, |crs| crs.to_projjson())
    }

    /// Build a [`Transformer`] bound to `context`.
    pub fn build<'context>(&self, context: &'context Context) -> Result<Transformer<'context>> {
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

    /// Transform a single coordinate of any [`Coord`] type.
    ///
    /// The one scalar entry point. Works with [`Coord2`](crate::Coord2),
    /// [`Coord3`](crate::Coord3), [`Coord4`](crate::Coord4), tuples, and your
    /// own types: it reads `x`/`y`/`z`/`t` through the [`Coord`] trait and
    /// rebuilds via [`Coord::from_xyzt`], with no allocation and no boxing. The
    /// dimensionality of the coordinate you pass determines the dimensionality
    /// of the transform - a [`Coord2`](crate::Coord2) reports `z = 0.0` and
    /// `t = INFINITY`, which is exactly what PROJ expects for 2D.
    ///
    /// ```no_run
    /// # use proxi::{Context, Coord3, Direction, AngularUnits, TransformerBuilder};
    /// # fn main() -> proxi::Result<()> {
    /// # let context = Context::new()?;
    /// # let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857").build()?;
    /// let out = t.transform(
    ///     Coord3::new(-71.1, 42.3, 0.0),
    ///     Direction::Forward,
    ///     AngularUnits::Auto,
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// For many coordinates, use [`transform_coords`](Self::transform_coords),
    /// which batches them into a single PROJ call.
    pub fn transform<C: Coord>(
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

    /// Transform a slice of coordinates of any [`Coord`] type in place.
    ///
    /// This is the one entry point for transforming many coordinates that live
    /// in a slice, whatever their layout: [`Coord3`](crate::Coord3), tuples,
    /// interleaved `[[f64; 3]]`, `glam::DVec3`, `geo::Point`, or your own type.
    /// All of them go through a single `proj_trans_generic` call rather than one
    /// PROJ call per point.
    ///
    /// The slice is scattered into the transformer's reusable scratch buffers,
    /// transformed in one call, then gathered back. The buffers grow
    /// geometrically and never shrink, so after warmup this allocates nothing.
    /// Only the columns the type actually uses are moved, as declared by
    /// [`Coord::DIMS`].
    ///
    /// If your data already lives in *separate* `x`/`y`/`z` buffers, prefer
    /// [`transform_batch`](Self::transform_batch): it hands those buffers
    /// straight to PROJ with no scatter/gather at all.
    ///
    /// ```no_run
    /// # use proxi::{Atomicity, Context, Coord3, Direction, AngularUnits, TransformerBuilder};
    /// # fn main() -> proxi::Result<()> {
    /// # let context = Context::new()?;
    /// # let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857").build()?;
    /// let mut points = [Coord3::new(-71.1, 42.3, 0.0), Coord3::new(-0.1, 51.5, 0.0)];
    /// t.transform_coords(
    ///     &mut points,
    ///     Direction::Forward,
    ///     AngularUnits::Auto,
    ///     Atomicity::All,
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn transform_coords<C: Coord>(
        &mut self,
        coords: &mut [C],
        direction: Direction,
        units: AngularUnits,
        atomicity: Atomicity,
    ) -> Result<usize> {
        let n = coords.len();
        if n == 0 {
            return Ok(0);
        }
        // Const-evaluated so the unused-column branches below fold away
        // entirely for each concrete `C`, exactly as a `const N` would.
        const {
            assert!(
                C::DIMS >= 2 && C::DIMS <= 4,
                "Coord::DIMS must be 2, 3, or 4"
            )
        }
        let has_z = const { C::DIMS > 2 };
        let has_t = const { C::DIMS > 3 };
        self.scratch.ensure_capacity(n);

        // Zip rather than index: the slices are known equal-length, so this
        // scatters with no per-element bounds check.
        let (xs, ys, zs, ts) = self.scratch.split_prefix(n);
        for (((c, x), y), (z, t)) in coords
            .iter()
            .zip(xs.iter_mut())
            .zip(ys.iter_mut())
            .zip(zs.iter_mut().zip(ts.iter_mut()))
        {
            *x = c.x();
            *y = c.y();
            if has_z {
                *z = c.z();
            }
            if has_t {
                *t = c.t();
            }
        }

        let (xs, ys, zs, ts) = self.scratch.split_prefix(n);
        let result = transform_slices_soa_partial(
            &self.obj,
            self.context,
            direction,
            units,
            &self.units,
            xs,
            ys,
            has_z.then_some(zs),
            has_t.then_some(ts),
        );

        let processed = match (result, atomicity) {
            (Ok(processed), _) => processed,
            // Nothing has been written back to the caller's slice yet, so
            // returning here *is* the rollback: the scratch data is discarded.
            (Err(failure), Atomicity::All) => return Err(failure.into()),
            (Err(failure), Atomicity::Partial) => failure.processed,
        };

        // Gather the committed prefix back into the caller's slice. Components
        // the type does not carry are supplied from the trait defaults so
        // `from_xyzt` always sees what PROJ would have produced.
        let (xs, ys, zs, ts) = self.scratch.split_prefix(processed);
        for (((c, &x), &y), (&z, &t)) in coords[..processed]
            .iter_mut()
            .zip(xs.iter())
            .zip(ys.iter())
            .zip(zs.iter().zip(ts.iter()))
        {
            let z = if has_z { z } else { c.z() };
            let t = if has_t { t } else { c.t() };
            *c = C::from_xyzt(x, y, z, t);
        }
        Ok(processed)
    }

    /// Transform a slice of coordinates in place, skipping non-finite points.
    ///
    /// Identical to [`transform_coords`](Self::transform_coords) with
    /// [`Atomicity::Partial`], except that points whose `x` or `y` is `NaN` or
    /// infinite are never handed to PROJ. Such a point counts toward the batch
    /// total but not toward `processed`, and its value is left unchanged.
    ///
    /// `z` is carried through even when non-finite: height is normally the
    /// vertical component rather than the angular driver, so a missing height
    /// should not invalidate an otherwise-good horizontal coordinate.
    ///
    /// The first non-finite point truncates the processed prefix, so the
    /// returned count is at most the index of the first bad point.
    pub fn transform_coords_finite<C: Coord>(
        &mut self,
        coords: &mut [C],
        direction: Direction,
        units: AngularUnits,
    ) -> Result<usize> {
        let total = coords.len();
        let first_bad = coords
            .iter()
            .position(|c| !c.x().is_finite() || !c.y().is_finite())
            .unwrap_or(total);
        let processed = self.transform_coords(
            &mut coords[..first_bad],
            direction,
            units,
            Atomicity::Partial,
        )?;
        if processed == total {
            Ok(processed)
        } else {
            Err(crate::errors::PartialFailure { processed, total }.into())
        }
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
                if !grid.is_available()
                    && grid.direct_download
                    && let Some(url) = grid.url
                {
                    self.context.download_grid(&url)?;
                    downloaded += 1;
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

    /// Transform a structure-of-arrays batch in place.
    ///
    /// The primary batch entry point and the fastest path in the crate: a
    /// [`CoordBatch`] borrows the caller's `x`/`y` (and optionally `z`/`t`)
    /// buffers and hands them straight to `proj_trans_generic`, so with
    /// [`Atomicity::Partial`] there is no copy and no allocation at all.
    ///
    /// The dimensionality comes from the batch - build it with
    /// [`CoordBatch::new`] and add [`with_z`](CoordBatch::with_z) /
    /// [`with_t`](CoordBatch::with_t) as needed - so there is one method rather
    /// than one per dimension.
    ///
    /// Returns the number of coordinates transformed. Under
    /// [`Atomicity::All`] (the default) that always equals the batch length on
    /// success, and the caller's buffers are untouched on failure.
    ///
    /// ```no_run
    /// # use proxi::{Atomicity, Context, CoordBatch, Direction, AngularUnits, TransformerBuilder};
    /// # fn main() -> proxi::Result<()> {
    /// # let context = Context::new()?;
    /// # let mut t = TransformerBuilder::new(&context, "EPSG:4326", "EPSG:3857").build()?;
    /// let mut xs = [-71.1, -0.1];
    /// let mut ys = [42.3, 51.5];
    /// let mut zs = [0.0, 0.0];
    ///
    /// let batch = CoordBatch::new(&mut xs, &mut ys)?.with_z(&mut zs)?;
    /// t.transform_batch(batch, Direction::Forward, AngularUnits::Auto, Atomicity::All)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn transform_batch(
        &mut self,
        coords: CoordBatch<'_>,
        direction: Direction,
        units: AngularUnits,
        atomicity: Atomicity,
    ) -> Result<usize> {
        let (x, y, z, t) = coords.into_parts();
        // Defensive check at the FFI boundary: `CoordBatch`'s validating
        // constructors enforce equal lengths, but re-verify here so the safety
        // argument is local to the one place that hands slices to PROJ.
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

        let n = x.len();
        if n == 0 {
            return Ok(0);
        }

        match atomicity {
            // Zero-copy: PROJ writes directly into the caller's buffers and a
            // short count is reported rather than rolled back.
            Atomicity::Partial => match transform_slices_soa_partial(
                &self.obj,
                self.context,
                direction,
                units,
                &self.units,
                x,
                y,
                z,
                t,
            ) {
                Ok(processed) => Ok(processed),
                Err(failure) => Ok(failure.processed),
            },
            // All-or-nothing: stage through scratch so the caller's buffers are
            // only written once PROJ has committed the whole batch.
            Atomicity::All => {
                self.scratch.ensure_capacity(n);
                self.scratch.xs[..n].copy_from_slice(x);
                self.scratch.ys[..n].copy_from_slice(y);
                if let Some(z) = z.as_deref() {
                    self.scratch.zs[..n].copy_from_slice(z);
                }
                if let Some(t) = t.as_deref() {
                    self.scratch.ts[..n].copy_from_slice(t);
                }

                let has_z = z.is_some();
                let has_t = t.is_some();
                let (sx, sy, sz, st) = self.scratch.split_prefix(n);
                transform_slices_soa_partial(
                    &self.obj,
                    self.context,
                    direction,
                    units,
                    &self.units,
                    sx,
                    sy,
                    has_z.then_some(sz),
                    has_t.then_some(st),
                )?;

                x.copy_from_slice(&self.scratch.xs[..n]);
                y.copy_from_slice(&self.scratch.ys[..n]);
                if let Some(z) = z {
                    z.copy_from_slice(&self.scratch.zs[..n]);
                }
                if let Some(t) = t {
                    t.copy_from_slice(&self.scratch.ts[..n]);
                }
                Ok(n)
            }
        }
    }
}

/// Whether a batch transform may commit a partial result.
///
/// PROJ can stop part-way through a batch. This selects what happens to the
/// caller's buffers when it does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Atomicity {
    /// All-or-nothing: on a short count the caller's buffers are left exactly
    /// as they were and [`ProxiError::Partial`] is returned.
    ///
    /// For [`transform_batch`](Transformer::transform_batch) this requires
    /// staging through the transformer's internal scratch buffers, which costs
    /// two extra copies over the batch. It is the default because silently
    /// half-transformed data is the more dangerous failure mode.
    #[default]
    All,
    /// Best-effort: the successfully transformed prefix is committed and the
    /// number of transformed coordinates is returned. A short count is *not* an
    /// error - compare the returned count against the batch length.
    ///
    /// This is the fastest option: for
    /// [`transform_batch`](Transformer::transform_batch) PROJ writes directly
    /// into the caller's buffers with no staging copy.
    Partial,
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
fn network_can_download(context: &Context) -> bool {
    #[cfg(feature = "network")]
    {
        context.network_enabled()
    }
    #[cfg(not(feature = "network"))]
    {
        let _ = context;
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
    #[cfg_attr(not(feature = "network"), allow(unused_mut))]
    let mut grids = ffi::operation_grids(obj, context)?;
    let missing = || grids.iter().filter(|grid| !grid.is_available()).count();
    if missing() == 0 {
        return Ok(());
    }
    if options.grid_policy == GridPolicy::DownloadMissing && network_can_download(context) {
        // `download_grid` only exists with the `network` feature; without it
        // `network_can_download` is a constant `false`, so this block is dead
        // code that must nonetheless still compile.
        #[cfg(feature = "network")]
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
    input: &str,
    serialize: impl FnOnce(&Crs<'context>) -> Result<T>,
) -> Result<T> {
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

/// Core SOA transform that reports a short PROJ count as a partial failure.
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
    let n_out = ffi::trans_generic(obj, dir, x, y, z, t).min(n);
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
