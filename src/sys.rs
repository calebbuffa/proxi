//! Raw FFI bindings for the full PROJ public API surface.
//!
//! Exposed publicly as `proxi::sys` so power users can reach any PROJ symbol
//! directly (like `proj_sys` / `libc`). Contents are generated from the pinned
//! PROJ headers (`proj.h` + `geodesic.h` + `proj_experimental.h`, including the
//! database-query surface that lives in `proj.h`) via
//! `scripts/regenerate-bindings.sh`, and the output is committed.
//!
//! The ergonomic safe API is unchanged: internal `mod bindings` re-exports this
//! module, so every existing `bindings::*` reference keeps resolving. The small
//! appendix below provides the flat C-style enum constants PROJ exposes, the
//! `PJ_COORD` constructor helpers, and the crate's own `proxi_*` C shim
//! declarations (`native/shim.c`) — all legitimate raw-surface items, not a
//! compatibility layer.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(dead_code)]
#![allow(improper_ctypes)]

/// The full bindgen output for the pinned PROJ headers.
#[path = "bindings.rs"]
mod raw;

pub use raw::*;

// Flat C-style enum constants that PROJ exposes and the crate's internals
// reference via `bindings::CONST`. They alias the concrete, type-typed values
// emitted by bindgen (default `consts` style), so callers can write the plain
// PROJ constant names while keeping FFI-safe `c_int`-backed enum types.

/// Forward direction.
pub const PJ_FWD: PJ_DIRECTION = PJ_DIRECTION_PJ_FWD;
/// Identity direction.
pub const PJ_IDENT: PJ_DIRECTION = PJ_DIRECTION_PJ_IDENT;
/// Inverse direction.
pub const PJ_INV: PJ_DIRECTION = PJ_DIRECTION_PJ_INV;
/// WKT2:2015 output.
pub const PJ_WKT2_2015: PJ_WKT_TYPE = PJ_WKT_TYPE_PJ_WKT2_2015;
/// WKT2:2019 output.
pub const PJ_WKT2_2019: PJ_WKT_TYPE = PJ_WKT_TYPE_PJ_WKT2_2019;
/// WKT1 (GDAL) output.
pub const PJ_WKT1_GDAL: PJ_WKT_TYPE = PJ_WKT_TYPE_PJ_WKT1_GDAL;
/// WKT1 (ESRI) output.
pub const PJ_WKT1_ESRI: PJ_WKT_TYPE = PJ_WKT_TYPE_PJ_WKT1_ESRI;
/// CRS comparison: strict.
pub const PJ_COMP_STRICT: PJ_COMPARISON_CRITERION = PJ_COMPARISON_CRITERION_PJ_COMP_STRICT;
/// CRS comparison: equivalent.
pub const PJ_COMP_EQUIVALENT: PJ_COMPARISON_CRITERION = PJ_COMPARISON_CRITERION_PJ_COMP_EQUIVALENT;
/// CRS comparison: equivalent except axis order.
pub const PJ_COMP_EQUIVALENT_EXCEPT_AXIS_ORDER_GEOGCRS: PJ_COMPARISON_CRITERION =
    PJ_COMPARISON_CRITERION_PJ_COMP_EQUIVALENT_EXCEPT_AXIS_ORDER_GEOGCRS;
/// Cartesian coordinate-system type.
pub const PJ_CS_TYPE_CARTESIAN: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_CARTESIAN;
/// Ellipsoidal coordinate-system type.
pub const PJ_CS_TYPE_ELLIPSOIDAL: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_ELLIPSOIDAL;
/// Vertical coordinate-system type.
pub const PJ_CS_TYPE_VERTICAL: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_VERTICAL;
/// Spherical coordinate-system type.
pub const PJ_CS_TYPE_SPHERICAL: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_SPHERICAL;
/// Ordinal coordinate-system type.
pub const PJ_CS_TYPE_ORDINAL: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_ORDINAL;
/// Parametric coordinate-system type.
pub const PJ_CS_TYPE_PARAMETRIC: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_PARAMETRIC;
/// Date-time temporal coordinate-system type.
pub const PJ_CS_TYPE_DATETIME_TEMPORAL: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_DATETIMETEMPORAL;
/// Temporal-count coordinate-system type.
pub const PJ_CS_TYPE_TEMPORAL_COUNT: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_TEMPORALCOUNT;
/// Temporal-measure coordinate-system type.
pub const PJ_CS_TYPE_TEMPORAL_MEASURE: PJ_COORDINATE_SYSTEM_TYPE =
    PJ_COORDINATE_SYSTEM_TYPE_PJ_CS_TYPE_TEMPORALMEASURE;
/// Object category: CRS.
pub const PJ_CATEGORY_CRS: PJ_CATEGORY = PJ_CATEGORY_PJ_CATEGORY_CRS;
/// Object type: CRS.
pub const PJ_TYPE_CRS: PJ_TYPE = PJ_TYPE_PJ_TYPE_CRS;
/// Object type: datum ensemble.
pub const PJ_TYPE_DATUM_ENSEMBLE: PJ_TYPE = PJ_TYPE_PJ_TYPE_DATUM_ENSEMBLE;
/// Object type: concatenated operation.
pub const PJ_TYPE_CONCATENATED_OPERATION: PJ_TYPE = PJ_TYPE_PJ_TYPE_CONCATENATED_OPERATION;
/// Object type: other coordinate operation.
pub const PJ_TYPE_OTHER_COORDINATE_OPERATION: PJ_TYPE = PJ_TYPE_PJ_TYPE_OTHER_COORDINATE_OPERATION;
/// Object type: conversion.
pub const PJ_TYPE_CONVERSION: PJ_TYPE = PJ_TYPE_PJ_TYPE_CONVERSION;
/// Object type: transformation.
pub const PJ_TYPE_TRANSFORMATION: PJ_TYPE = PJ_TYPE_PJ_TYPE_TRANSFORMATION;

/// Convenience constructors for the raw [`PJ_COORD`] union.
///
/// These initialize the 4-tuple backing storage (`v`), which every named view
/// aliases, so callers need not select a variant just to pass a coordinate.
impl PJ_COORD {
    /// Build a coordinate from `(x, y, z, t)`.
    pub fn xyzt(x: f64, y: f64, z: f64, t: f64) -> Self {
        Self { v: [x, y, z, t] }
    }

    /// Build a coordinate from `(longitude, latitude, height)` (no time).
    pub fn lpz(lam: f64, phi: f64, z: f64) -> Self {
        Self {
            v: [lam, phi, z, f64::INFINITY],
        }
    }
}
