//! # proxi
//!
//! High-performance, user-friendly Rust bindings for [PROJ](https://proj.org).
//!
//! ## Example
//!
//! ```ignore
//! use proxi::{Context, Coord3, TransformerBuilder};
//!
//! let context = Context::new()?;
//! let mut t = TransformerBuilder::new(&context, "EPSG:4978", "EPSG:26986+5773")
//!     .always_xy(true)
//!     .build()?;
//! let out = t.forward_xyz(Coord3::new(1113194.9079327357, -4849539.959443408, 3987474.1413001378))?;
//! ```

mod context;
mod coord;
mod crs;
mod database;
mod embedded_data;
mod errors;
mod ffi;
mod geod;
mod options;
mod scratch;
mod transform;
mod version;

pub mod sys;
/// Internal alias so the ergonomic modules keep `use crate::bindings::*;`
/// unchanged while the raw surface lives under the public `proxi::sys`.
mod bindings {
    pub(crate) use crate::sys::*;
}

pub use context::{Context, ContextDataPaths};
pub use coord::{Coord, Coord2, Coord3, Coord4, CoordBatch};
pub use crs::{
    AxisInfo, Conversion, CoordinateSystem, CoordinateSystemBuilder, CoordinateSystemType, Crs,
    CrsComparison, CrsIdentifier, CrsInfo, CrsType, DEGREE_METRE, DatumEnsembleInfo,
    EllipsoidParameters, PrimeMeridianParameters, Proj, Units,
};
pub use database::{
    CrsInfoRecord, CrsSearch, Database, DatabaseType, Ellipsoid, Operation, PrimeMeridian, Unit,
    UnitRecord,
};
pub use errors::{PartialFailure, ProxiError, Result};
pub use geod::{
    Geod, GeodesicDirect, GeodesicDirectIndexed, GeodesicInverse, GeodesicInverseIndexed,
    GeodesicPolygon, Line, LineCaps, LinePosition, PolygonBuilder,
};
pub use options::{
    AngularUnits, AreaOfInterest, AreaOfUse, AxisOutputOrder, ContextOptions, Direction,
    GridPolicy, ProjStringVersion, WktOptions, WktVersion,
};
pub use transform::{
    CrsExtentUse, GridAvailabilityUse, GridInfo, GridReadiness, GridReport, OperationInfo,
    OperationParameter, SpatialCriterion, Transformer, TransformerBuilder, TransformerDefinition,
    TransformerGroup, TransformerGroupBuilder,
};
pub use version::{
    BUILT_VERSION_MAJOR, BUILT_VERSION_MINOR, BUILT_VERSION_PATCH, ProjVersion,
    check_runtime_compatibility,
};
