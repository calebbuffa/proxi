//! Read-only queries against the configured PROJ database.

use crate::context::Context;
use crate::crs::Crs;
use crate::errors::Result;
use crate::ffi;

/// Read-only access to the PROJ database associated with a context.
pub struct Database<'context> {
    context: &'context Context,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ellipsoid {
    pub id: String,
    pub major: String,
    pub ellipsoid: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub id: String,
    pub to_meter: String,
    pub name: String,
    pub factor: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrimeMeridian {
    pub id: String,
    pub definition: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation {
    pub id: String,
    pub description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseType {
    Crs,
    DatumEnsemble,
    ConcatenatedOperation,
    OtherCoordinateOperation,
}

/// A structured record returned by a filtered CRS search.
#[derive(Clone, Debug, PartialEq)]
pub struct CrsInfoRecord {
    pub authority: Option<String>,
    pub code: Option<String>,
    pub name: Option<String>,
    pub r#type: crate::crs::CrsType,
    pub area_name: Option<String>,
    pub west_lon_degree: f64,
    pub south_lat_degree: f64,
    pub east_lon_degree: f64,
    pub north_lat_degree: f64,
    pub bbox_valid: bool,
    pub deprecated: bool,
    pub celestial_body_name: Option<String>,
}

/// A structured unit record from the database.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitRecord {
    pub authority: Option<String>,
    pub code: Option<String>,
    pub name: Option<String>,
    pub category: Option<String>,
    pub conversion_factor: f64,
    pub proj_short_name: Option<String>,
    pub deprecated: bool,
}

/// Filter for a [`Database::crs_search`] query.
#[derive(Clone, Debug, Default)]
pub struct CrsSearch {
    pub types: Vec<crate::crs::CrsType>,
    pub allow_deprecated: bool,
    pub area: Option<[f64; 4]>,
    pub celestial_body: Option<String>,
}

impl<'context> Database<'context> {
    /// Open the database associated with an existing thread-bound context.
    pub fn new(context: &'context Context) -> Self {
        Self { context }
    }

    /// List authorities present in the active PROJ database.
    pub fn authorities(&self) -> Vec<String> {
        ffi::database_authorities(self.context)
    }

    /// List CRS codes for an authority, excluding deprecated codes.
    pub fn codes(&self, authority: &str) -> Result<Vec<String>> {
        ffi::database_codes(self.context, authority)
    }

    pub fn codes_of_type(&self, authority: &str, object_type: DatabaseType) -> Result<Vec<String>> {
        ffi::database_codes_of_type(self.context, authority, object_type)
    }

    /// Construct a CRS from an authority and code using this database context.
    pub fn crs(&self, authority: &str, code: &str) -> Result<Crs<'context>> {
        Crs::from_authority(self.context, authority, code)
    }

    pub fn ellipsoids(&self) -> Vec<Ellipsoid> {
        ffi::database_ellipsoids()
    }

    pub fn units(&self) -> Vec<Unit> {
        ffi::database_units(false)
    }

    pub fn angular_units(&self) -> Vec<Unit> {
        ffi::database_units(true)
    }

    pub fn prime_meridians(&self) -> Vec<PrimeMeridian> {
        ffi::database_prime_meridians()
    }

    pub fn operations(&self) -> Vec<Operation> {
        ffi::database_operations()
    }

    /// Run a filtered CRS search against the database.
    ///
    /// `authority` restricts to one authority (or all when `None`); `filter`
    /// applies CRS-type, deprecated, area-of-use and celestial-body criteria.
    /// Returns structured records (name, type, bbox, deprecated flag).
    pub fn crs_search(
        &self,
        authority: Option<&str>,
        filter: &CrsSearch,
    ) -> Result<Vec<CrsInfoRecord>> {
        ffi::database_crs_info_list(
            self.context,
            authority,
            &types_to_pj(filter.types.as_slice()),
            filter.allow_deprecated,
            filter.area,
            filter.celestial_body.as_deref(),
        )
    }

    /// List units (linear/angular/... record) from the database, applying an
    /// optional authority and `allow_deprecated`.
    pub fn units_from_database(
        &self,
        authority: Option<&str>,
        category: &str,
        allow_deprecated: bool,
    ) -> Result<Vec<UnitRecord>> {
        ffi::database_units_from_database(self.context, authority, category, allow_deprecated)
    }

    /// List the geoid-model names associated with a geographic CRS
    /// (`authority:code`, e.g. `EPSG:4326`).
    pub fn geoid_models(&self, authority: &str, code: &str) -> Result<Vec<String>> {
        ffi::database_geoid_models(self.context, authority, code)
    }

    /// Resolve grid metadata (full name, package, URL, availability) for a
    /// named grid from the database.
    pub fn grid(&self, grid_name: &str) -> Result<crate::transform::GridInfo> {
        ffi::database_grid_info(self.context, grid_name)
    }
}

fn types_to_pj(types: &[crate::crs::CrsType]) -> Vec<crate::bindings::PJ_TYPE> {
    use crate::bindings::*;
    types
        .iter()
        .map(|t| match t {
            crate::crs::CrsType::Geographic2DCrs => PJ_TYPE_PJ_TYPE_GEOGRAPHIC_2D_CRS,
            crate::crs::CrsType::Geographic3DCrs => PJ_TYPE_PJ_TYPE_GEOGRAPHIC_3D_CRS,
            crate::crs::CrsType::ProjectedCrs => PJ_TYPE_PJ_TYPE_PROJECTED_CRS,
            crate::crs::CrsType::VerticalCrs => PJ_TYPE_PJ_TYPE_VERTICAL_CRS,
            crate::crs::CrsType::CompoundCrs => PJ_TYPE_PJ_TYPE_COMPOUND_CRS,
            crate::crs::CrsType::GeocentricCrs => PJ_TYPE_PJ_TYPE_GEOCENTRIC_CRS,
            crate::crs::CrsType::EngineeringCrs => PJ_TYPE_PJ_TYPE_ENGINEERING_CRS,
            // Fall back to the generic CRS type for anything else.
            _ => PJ_TYPE_PJ_TYPE_CRS,
        })
        .collect()
}
