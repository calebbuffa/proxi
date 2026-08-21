//! A thread-bound, owned coordinate reference system.

use crate::context::Context;
use crate::errors::Result;
use crate::ffi;
use crate::options::WktVersion;

/// A CRS authority identifier such as `EPSG:4326`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrsIdentifier {
    pub authority: String,
    pub code: String,
}

/// Owned descriptive metadata for a CRS.
#[derive(Clone, Debug, PartialEq)]
pub struct CrsInfo {
    pub name: Option<String>,
    pub identifiers: Vec<CrsIdentifier>,
    pub scope: Option<String>,
    pub remarks: Option<String>,
    pub area_of_use: Option<crate::options::AreaOfUse>,
    pub coordinate_system: Option<CoordinateSystem>,
    pub ellipsoid: Option<EllipsoidParameters>,
    pub prime_meridian: Option<PrimeMeridianParameters>,
    pub datum_ensemble: Option<DatumEnsembleInfo>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DatumEnsembleInfo {
    pub name: Option<String>,
    pub accuracy_meters: f64,
    pub members: Vec<Option<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EllipsoidParameters {
    pub semi_major_meters: f64,
    pub semi_minor_meters: f64,
    pub inverse_flattening: f64,
    pub semi_minor_was_computed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrimeMeridianParameters {
    pub longitude: f64,
    pub unit_conversion_factor: f64,
    pub unit_name: Option<String>,
}

/// Coordinate-system metadata associated with a CRS.
#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateSystem {
    pub kind: CoordinateSystemType,
    pub axes: Vec<AxisInfo>,
}

/// Coordinate-system family reported by PROJ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinateSystemType {
    Unknown,
    Cartesian,
    Ellipsoidal,
    Vertical,
    Spherical,
    Ordinal,
    Parametric,
    DateTimeTemporal,
    TemporalCount,
    TemporalMeasure,
}

/// One CRS axis and its unit metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisInfo {
    pub name: Option<String>,
    pub abbreviation: Option<String>,
    pub direction: Option<String>,
    pub unit_conversion_factor: f64,
    pub unit_name: Option<String>,
    pub unit_authority: Option<String>,
    pub unit_code: Option<String>,
}

/// Criteria for comparing two CRS definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrsComparison {
    Strict,
    Equivalent,
    EquivalentExceptAxisOrder,
}

/// The category/type of a PROJ object, from `proj_get_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrsType {
    Unknown,
    Ellipsoid,
    PrimeMeridian,
    GeodeticReferenceFrame,
    DynamicGeodeticReferenceFrame,
    VerticalReferenceFrame,
    DynamicVerticalReferenceFrame,
    DatumEnsemble,
    Crs,
    GeodeticCrs,
    GeocentricCrs,
    GeographicCrs,
    Geographic2DCrs,
    Geographic3DCrs,
    VerticalCrs,
    ProjectedCrs,
    CompoundCrs,
    TemporalCrs,
    EngineeringCrs,
    BoundCrs,
    OtherCrs,
    Conversion,
    Transformation,
    ConcatenatedOperation,
    OtherCoordinateOperation,
    TemporalDatum,
    EngineeringDatum,
    ParametricDatum,
    DerivedProjectedCrs,
    CoordinateMetadata,
}

impl CrsType {
    /// Convert a raw `bindings::PJ_TYPE` to a `CrsType`.
    pub(crate) fn from_raw(raw: crate::bindings::PJ_TYPE) -> Self {
        use crate::bindings::*;
        match raw {
            PJ_TYPE_PJ_TYPE_UNKNOWN => CrsType::Unknown,
            PJ_TYPE_PJ_TYPE_ELLIPSOID => CrsType::Ellipsoid,
            PJ_TYPE_PJ_TYPE_PRIME_MERIDIAN => CrsType::PrimeMeridian,
            PJ_TYPE_PJ_TYPE_GEODETIC_REFERENCE_FRAME => CrsType::GeodeticReferenceFrame,
            PJ_TYPE_PJ_TYPE_DYNAMIC_GEODETIC_REFERENCE_FRAME => {
                CrsType::DynamicGeodeticReferenceFrame
            }
            PJ_TYPE_PJ_TYPE_VERTICAL_REFERENCE_FRAME => CrsType::VerticalReferenceFrame,
            PJ_TYPE_PJ_TYPE_DYNAMIC_VERTICAL_REFERENCE_FRAME => {
                CrsType::DynamicVerticalReferenceFrame
            }
            PJ_TYPE_PJ_TYPE_DATUM_ENSEMBLE => CrsType::DatumEnsemble,
            PJ_TYPE_PJ_TYPE_CRS => CrsType::Crs,
            PJ_TYPE_PJ_TYPE_GEODETIC_CRS => CrsType::GeodeticCrs,
            PJ_TYPE_PJ_TYPE_GEOCENTRIC_CRS => CrsType::GeocentricCrs,
            PJ_TYPE_PJ_TYPE_GEOGRAPHIC_CRS => CrsType::GeographicCrs,
            PJ_TYPE_PJ_TYPE_GEOGRAPHIC_2D_CRS => CrsType::Geographic2DCrs,
            PJ_TYPE_PJ_TYPE_GEOGRAPHIC_3D_CRS => CrsType::Geographic3DCrs,
            PJ_TYPE_PJ_TYPE_VERTICAL_CRS => CrsType::VerticalCrs,
            PJ_TYPE_PJ_TYPE_PROJECTED_CRS => CrsType::ProjectedCrs,
            PJ_TYPE_PJ_TYPE_COMPOUND_CRS => CrsType::CompoundCrs,
            PJ_TYPE_PJ_TYPE_TEMPORAL_CRS => CrsType::TemporalCrs,
            PJ_TYPE_PJ_TYPE_ENGINEERING_CRS => CrsType::EngineeringCrs,
            PJ_TYPE_PJ_TYPE_BOUND_CRS => CrsType::BoundCrs,
            PJ_TYPE_PJ_TYPE_OTHER_CRS => CrsType::OtherCrs,
            PJ_TYPE_PJ_TYPE_CONVERSION => CrsType::Conversion,
            PJ_TYPE_PJ_TYPE_TRANSFORMATION => CrsType::Transformation,
            PJ_TYPE_PJ_TYPE_CONCATENATED_OPERATION => CrsType::ConcatenatedOperation,
            PJ_TYPE_PJ_TYPE_OTHER_COORDINATE_OPERATION => CrsType::OtherCoordinateOperation,
            PJ_TYPE_PJ_TYPE_TEMPORAL_DATUM => CrsType::TemporalDatum,
            PJ_TYPE_PJ_TYPE_ENGINEERING_DATUM => CrsType::EngineeringDatum,
            PJ_TYPE_PJ_TYPE_PARAMETRIC_DATUM => CrsType::ParametricDatum,
            PJ_TYPE_PJ_TYPE_DERIVED_PROJECTED_CRS => CrsType::DerivedProjectedCrs,
            PJ_TYPE_PJ_TYPE_COORDINATE_METADATA => CrsType::CoordinateMetadata,
            _ => CrsType::Unknown,
        }
    }
}

/// A coordinate reference system, safe to use for WKT / PROJJSON output.
///
/// A `Crs` owns a `PJ*` and a PROJ context; both are thread-bound (the type
/// is `!Send` / `!Sync`). It is *not* required for transformer construction
/// (which takes CRS strings); it exists for standalone inspection, such as
/// producing `.prj`-compatible WKT.
pub struct Crs<'context> {
    /// The CRS object. Dropped before `_context` (field order), so the context
    /// is still alive when `proj_destroy` runs.
    obj: ffi::ProjObj,
    /// Owned context; must outlive `obj`. Kept alive for the lifetime of `Crs`.
    context: &'context Context,
}

impl<'context> Crs<'context> {
    /// Create a CRS from any PROJ user input (EPSG code, WKT, PROJ string).
    ///
    /// The definition must resolve to a CRS (verified via `proj_is_crs`);
    /// projection or operation strings are rejected.
    pub fn from_user_input(context: &'context Context, input: &str) -> Result<Self> {
        let obj = ffi::create_crs(context, input)?;
        Ok(Self { obj, context })
    }

    /// Create a CRS directly from an authority and database code.
    pub fn from_authority(context: &'context Context, authority: &str, code: &str) -> Result<Self> {
        let obj = ffi::create_crs_from_database(context, authority, code)?;
        Ok(Self { obj, context })
    }

    /// Build a geographic CRS from datum, ellipsoid, and prime-meridian parameters.
    /// Returns `None` when PROJ rejects the definition.
    ///
    /// `inverse_flattening` may be `0` for a sphere.
    #[allow(clippy::too_many_arguments)]
    pub fn geographic(
        context: &'context Context,
        crs_name: &str,
        datum_name: &str,
        ellipsoid_name: &str,
        semi_major_metre: f64,
        inverse_flattening: f64,
        prime_meridian_name: &str,
        prime_meridian_offset: f64,
        pm_angular_units: &str,
        pm_units_conv: f64,
    ) -> Option<Self> {
        let obj = ffi::create_geographic_crs(
            context,
            crs_name,
            datum_name,
            ellipsoid_name,
            semi_major_metre,
            inverse_flattening,
            prime_meridian_name,
            prime_meridian_offset,
            pm_angular_units,
            pm_units_conv,
        )?;
        Some(Self { obj, context })
    }

    /// Build a geographic CRS from an explicit datum object and coordinate system.
    /// Both `datum` and `cs` are only read by PROJ.
    ///
    /// `datum` is typically the horizontal datum returned by
    /// [`Crs::horizontal_datum`]; `cs` is a coordinate system from [`Proj`].
    pub fn geographic_from_datum(
        context: &'context Context,
        crs_name: &str,
        datum: &Crs,
        cs: &CoordinateSystemBuilder,
    ) -> Option<Self> {
        let obj = ffi::create_geographic_crs_from_datum(context, crs_name, &datum.obj, cs.obj())?;
        Some(Self { obj, context })
    }

    /// Build a projected CRS from a geodetic CRS, conversion, and coordinate system.
    /// All inputs are only read by PROJ.
    pub fn projected(
        context: &'context Context,
        crs_name: &str,
        geodetic_crs: &Crs,
        conversion: &Conversion,
        cs: &CoordinateSystemBuilder,
    ) -> Option<Self> {
        let obj = ffi::create_projected_crs(
            context,
            crs_name,
            &geodetic_crs.obj,
            conversion.obj(),
            cs.obj(),
        )?;
        Some(Self { obj, context })
    }

    /// Build a vertical CRS from datum and linear-unit strings.
    pub fn vertical(
        context: &'context Context,
        crs_name: &str,
        datum_name: &str,
        linear_units: &str,
        linear_units_conv: f64,
    ) -> Option<Self> {
        let obj = ffi::create_vertical_crs(
            context,
            crs_name,
            datum_name,
            linear_units,
            linear_units_conv,
        )?;
        Some(Self { obj, context })
    }

    /// Build a compound horizontal and vertical CRS. Inputs are read-only.
    pub fn compound(
        context: &'context Context,
        crs_name: &str,
        horiz_crs: &Crs,
        vert_crs: &Crs,
    ) -> Option<Self> {
        let obj = ffi::create_compound_crs(context, crs_name, &horiz_crs.obj, &vert_crs.obj)?;
        Some(Self { obj, context })
    }

    /// Build an engineering CRS.
    pub fn engineering(context: &'context Context, crs_name: &str) -> Option<Self> {
        let obj = ffi::create_engineering_crs(context, crs_name)?;
        Some(Self { obj, context })
    }

    /// Build a bound CRS from a base CRS, hub CRS, and transformation.
    /// All three are only *read* by PROJ.
    pub fn bound(
        context: &'context Context,
        base_crs: &Crs,
        hub_crs: &Crs,
        transformation: &Conversion,
    ) -> Option<Self> {
        let obj =
            ffi::crs_create_bound_crs(context, &base_crs.obj, &hub_crs.obj, transformation.obj())?;
        Some(Self { obj, context })
    }

    /// Wrap an owned PROJ object as a `Crs` bound to `context`.
    ///
    /// Used internally to expose objects obtained by name or as CRS components.
    pub(crate) fn from_obj(context: &'context Context, obj: ffi::ProjObj) -> Self {
        Self { obj, context }
    }

    /// Serialize this CRS as WKT in the requested version, with PROJ's default
    /// formatting options.
    pub fn to_wkt(&self, version: WktVersion) -> Result<String> {
        self.to_wkt_with_options(version, None)
    }

    /// Serialize this CRS as WKT in the requested version, applying PROJ's
    /// WKT output options (multiline, indentation, axis order, always_xy, ...).
    pub fn to_wkt_with_options(
        &self,
        version: WktVersion,
        options: Option<&crate::options::WktOptions>,
    ) -> Result<String> {
        ffi::as_wkt(&self.obj, version, options)
    }

    /// Serialize this CRS as PROJJSON.
    pub fn to_projjson(&self) -> Result<String> {
        ffi::as_projjson(&self.obj)
    }

    /// Serialize this CRS as a PROJ string (`+proj=...`) in the WKT2-era form.
    pub fn to_proj_string(&self) -> Result<String> {
        self.to_proj_string_with_version(crate::options::ProjStringVersion::Proj5)
    }

    /// Serialize this CRS as a PROJ string in the requested format.
    pub fn to_proj_string_with_version(
        &self,
        version: crate::options::ProjStringVersion,
    ) -> Result<String> {
        ffi::as_proj_string(&self.obj, self.context, version)
    }

    /// Return the PROJ object type of this CRS.
    ///
    /// Maps `proj_get_type` to a safe [`CrsType`]. Useful to distinguish a
    /// projected vs. geographic vs. vertical CRS without string parsing.
    pub fn crs_type(&self) -> CrsType {
        // SAFETY: `proj_get_type` takes a live PJ and returns a `PJ_TYPE`.
        let raw = unsafe { crate::bindings::proj_get_type(self.obj.as_ptr()) };
        CrsType::from_raw(raw)
    }

    /// Return this CRS's geodetic CRS (the geodetic component of a projected or
    /// geographic CRS), or `None` if this CRS is not geodetic-composed.
    pub fn geodetic_crs(&self) -> Option<Crs<'context>> {
        ffi::crs_geodetic_crs(self.context, &self.obj).map(|obj| Crs {
            obj,
            context: self.context,
        })
    }

    /// Return this CRS's horizontal datum, or `None` if absent.
    pub fn horizontal_datum(&self) -> Option<Crs<'context>> {
        ffi::crs_horizontal_datum(self.context, &self.obj).map(|obj| Crs {
            obj,
            context: self.context,
        })
    }

    /// Return one sub-CRS of a compound CRS by index, or `None`.
    pub fn sub_crs(&self, index: usize) -> Option<Crs<'context>> {
        ffi::crs_sub_crs(self.context, &self.obj, index).map(|obj| Crs {
            obj,
            context: self.context,
        })
    }

    /// Return the coordinate operation attached to this (bound/derived) CRS, or
    /// `None` if this CRS has no attached operation.
    pub fn coordinate_operation(&self) -> Option<Crs<'context>> {
        ffi::crs_coord_operation(self.context, &self.obj).map(|obj| Crs {
            obj,
            context: self.context,
        })
    }

    /// Return this CRS's datum (for geodetic/vertical CRSs), or `None`.
    pub fn datum(&self) -> Option<Crs<'context>> {
        ffi::crs_datum(self.context, &self.obj).map(|obj| Crs {
            obj,
            context: self.context,
        })
    }

    /// Return owned descriptive and authority metadata for this CRS.
    pub fn info(&self) -> Result<CrsInfo> {
        let area =
            ffi::area_of_use(&self.obj, self.context)?.map(|(west, south, east, north, name)| {
                crate::options::AreaOfUse {
                    west_lon_degree: west,
                    south_lat_degree: south,
                    east_lon_degree: east,
                    north_lat_degree: north,
                    name,
                }
            });
        Ok(CrsInfo {
            name: ffi::crs_name(&self.obj),
            identifiers: ffi::crs_identifiers(&self.obj)
                .into_iter()
                .map(|(authority, code)| CrsIdentifier { authority, code })
                .collect(),
            scope: ffi::crs_scope(&self.obj),
            remarks: ffi::crs_remarks(&self.obj),
            area_of_use: area,
            coordinate_system: ffi::crs_coordinate_system(&self.obj, self.context).map(
                |(kind, axes)| CoordinateSystem {
                    kind: coordinate_system_type(kind),
                    axes: axes
                        .into_iter()
                        .map(
                            |(
                                name,
                                abbreviation,
                                direction,
                                unit_conversion_factor,
                                unit_name,
                                unit_authority,
                                unit_code,
                            )| AxisInfo {
                                name,
                                abbreviation,
                                direction,
                                unit_conversion_factor,
                                unit_name,
                                unit_authority,
                                unit_code,
                            },
                        )
                        .collect(),
                },
            ),
            ellipsoid: ffi::crs_ellipsoid(&self.obj, self.context).map(
                |(
                    semi_major_meters,
                    semi_minor_meters,
                    inverse_flattening,
                    semi_minor_was_computed,
                )| {
                    EllipsoidParameters {
                        semi_major_meters,
                        semi_minor_meters,
                        inverse_flattening,
                        semi_minor_was_computed,
                    }
                },
            ),
            prime_meridian: ffi::crs_prime_meridian(&self.obj, self.context).map(
                |(longitude, unit_conversion_factor, unit_name)| PrimeMeridianParameters {
                    longitude,
                    unit_conversion_factor,
                    unit_name,
                },
            ),
            datum_ensemble: ffi::crs_datum_ensemble(&self.obj, self.context).map(
                |(name, accuracy_meters, members)| DatumEnsembleInfo {
                    name,
                    accuracy_meters,
                    members,
                },
            ),
        })
    }

    /// Compare this CRS with another CRS using PROJ's semantic criteria.
    pub fn equivalent_to(&self, other: &Crs, comparison: CrsComparison) -> bool {
        let criterion = match comparison {
            CrsComparison::Strict => crate::bindings::PJ_COMP_STRICT,
            CrsComparison::Equivalent => crate::bindings::PJ_COMP_EQUIVALENT,
            CrsComparison::EquivalentExceptAxisOrder => {
                crate::bindings::PJ_COMP_EQUIVALENT_EXCEPT_AXIS_ORDER_GEOGCRS
            }
        };
        ffi::crs_equivalent(self.context, &self.obj, &other.obj, criterion)
    }
}

/// A datum or coordinate-system object used to assemble a [`Crs`].
///
/// These are owned, context-bound `PJ*` objects. PROJ's composite constructors
/// (`proj_create_geographic_crs_from_datum`, `proj_create_projected_crs`, ...)
/// only *read* the datum / coordinate system they are given, so the CS/datum
/// lives as long as (or longer than) the call — here it is an owned value the
/// caller must keep alive while the CRS is built.
pub struct Proj<'context> {
    obj: ffi::ProjObj,
    // Kept for thread-affinity: guarantees the PROJ object does not outlive
    // its owning context (dropping a PJ after its context is invalid).
    #[allow(dead_code)]
    context: &'context Context,
}

impl<'context> Proj<'context> {
    /// Create the standard 2D ellipsoidal (geographic) coordinate system.
    ///
    /// `longitude_latitude` selects the longitude/latitude axis order; the
    /// alternative is latitude/longitude. `unit_name` is e.g. `"degree"` with
    /// `unit_conv_factor` = 0.0174532925199433 (radians per degree).
    pub fn ellipsoidal_2d_cs(
        context: &'context Context,
        longitude_latitude: bool,
        unit_name: &str,
        unit_conv_factor: f64,
    ) -> Option<Self> {
        let obj = ffi::create_ellipsoidal_2d_cs(
            context,
            longitude_latitude,
            unit_name,
            unit_conv_factor,
        )?;
        Some(Self { obj, context })
    }

    /// Create a 2D cartesian coordinate system (easting/northing or the
    /// northing/easting variant).
    pub fn cartesian_2d_cs(
        context: &'context Context,
        easting_northing: bool,
        unit_name: &str,
        unit_conv_factor: f64,
    ) -> Option<Self> {
        let obj =
            ffi::create_cartesian_2d_cs(context, easting_northing, unit_name, unit_conv_factor)?;
        Some(Self { obj, context })
    }

    /// Create a 3D ellipsoidal (geographic + height) coordinate system.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipsoidal_3d_cs(
        context: &'context Context,
        longitude_latitude_height: bool,
        horizontal_unit_name: &str,
        horizontal_unit_conv_factor: f64,
        vertical_unit_name: &str,
        vertical_unit_conv_factor: f64,
    ) -> Option<Self> {
        let obj = ffi::create_ellipsoidal_3d_cs(
            context,
            longitude_latitude_height,
            horizontal_unit_name,
            horizontal_unit_conv_factor,
            vertical_unit_name,
            vertical_unit_conv_factor,
        )?;
        Some(Self { obj, context })
    }

    /// The underlying owned PROJ object (borrowed for the composite call).
    pub(crate) fn obj(&self) -> &ffi::ProjObj {
        &self.obj
    }
}

/// Alias for ergonomics: a coordinate-system builder is a [`Proj`].
pub type CoordinateSystemBuilder<'context> = Proj<'context>;

/// A map projection conversion, used to assemble a projected [`Crs`].
///
/// Owns a PROJ `CONVERSION` object bound to a context. `Conversion` values are
/// cheap to build via the constructors below and are passed (read-only) to
/// [`Crs::projected`] / [`Crs::bound`].
pub struct Conversion<'context> {
    obj: ffi::ProjObj,
    // Kept for thread-affinity: guarantees the PROJ object does not outlive
    // its owning context (dropping a PJ after its context is invalid).
    #[allow(dead_code)]
    context: &'context Context,
}

/// The standard angular/linear unit pair used by PROJ conversion constructors.
#[derive(Clone, Copy, Debug)]
pub struct Units {
    pub ang_name: &'static str,
    pub ang_conv: f64,
    pub linear_name: &'static str,
    pub linear_conv: f64,
}

/// Conventional degree / metre unit pair (radians-per-degree conversion).
pub const DEGREE_METRE: Units = Units {
    ang_name: "degree",
    ang_conv: 0.0174532925199433,
    linear_name: "metre",
    linear_conv: 1.0,
};

macro_rules! conversion_builder {
    ($(#[$doc:meta])* $name:ident => $ffi:path; $( $arg:ident : $ty:ty ),* $(,)?) => {
        $(#[$doc])*
        #[allow(clippy::too_many_arguments)]
        pub fn $name(
            context: &'context Context,
            units: Units,
            $( $arg: $ty, )*
        ) -> Option<Self> {
            let obj = $ffi(
                context,
                $( $arg, )*
                units.ang_name,
                units.ang_conv,
                units.linear_name,
                units.linear_conv,
            )?;
            Some(Self::wrap(context, obj))
        }
    };
}

/// For conversions that take only an angular unit pair (pole rotations).
macro_rules! conversion_ang_builder {
    ($(#[$doc:meta])* $name:ident => $ffi:path; $( $arg:ident : $ty:ty ),* $(,)?) => {
        $(#[$doc])*
        pub fn $name(
            context: &'context Context,
            ang_name: &str,
            ang_conv: f64,
            $( $arg: $ty, )*
        ) -> Option<Self> {
            let obj = $ffi(
                context,
                $( $arg, )*
                ang_name,
                ang_conv,
            )?;
            Some(Self::wrap(context, obj))
        }
    };
}

impl<'context> Conversion<'context> {
    fn wrap(context: &'context Context, obj: ffi::ProjObj) -> Self {
        Self { obj, context }
    }

    pub(crate) fn obj(&self) -> &ffi::ProjObj {
        &self.obj
    }

    /// UTM conversion (zone, north/south hemisphere).
    pub fn utm(context: &'context Context, zone: i32, north: bool) -> Option<Self> {
        ffi::conversion_utm(context, zone, north).map(|obj| Self::wrap(context, obj))
    }

    conversion_builder! {
        /// Transverse Mercator.
        transverse_mercator => ffi::conversion_transverse_mercator;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Gauss–Schreiber Transverse Mercator.
        gauss_schreiber_transverse_mercator => ffi::conversion_gauss_schreiber_transverse_mercator;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Transverse Mercator, south oriented.
        transverse_mercator_south_oriented => ffi::conversion_transverse_mercator_south_oriented;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Two-point equidistant.
        two_point_equidistant => ffi::conversion_two_point_equidistant;
        latitude_first_point: f64, longitude_first_point: f64,
        latitude_second_point: f64, longitude_secon_point: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Tunisia mapping grid.
        tunisia_mapping_grid => ffi::conversion_tunisia_mapping_grid;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Tunisia mining grid.
        tunisia_mining_grid => ffi::conversion_tunisia_mining_grid;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Albers equal area.
        albers_equal_area => ffi::conversion_albers_equal_area;
        latitude_false_origin: f64, longitude_false_origin: f64,
        latitude_first_parallel: f64, latitude_second_parallel: f64,
        easting_false_origin: f64, northing_false_origin: f64,
    }
    conversion_builder! {
        /// Lambert Conic Conformal (1SP).
        lambert_conic_conformal_1sp => ffi::conversion_lambert_conic_conformal_1sp;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Lambert Conic Conformal (1SP, variant B).
        lambert_conic_conformal_1sp_variant_b => ffi::conversion_lambert_conic_conformal_1sp_variant_b;
        latitude_nat_origin: f64, scale: f64, latitude_false_origin: f64,
        longitude_false_origin: f64, easting_false_origin: f64, northing_false_origin: f64,
    }
    conversion_builder! {
        /// Lambert Conic Conformal (2SP).
        lambert_conic_conformal_2sp => ffi::conversion_lambert_conic_conformal_2sp;
        latitude_false_origin: f64, longitude_false_origin: f64,
        latitude_first_parallel: f64, latitude_second_parallel: f64,
        easting_false_origin: f64, northing_false_origin: f64,
    }
    conversion_builder! {
        /// Lambert Conic Conformal (2SP, Michigan variant).
        lambert_conic_conformal_2sp_michigan => ffi::conversion_lambert_conic_conformal_2sp_michigan;
        latitude_false_origin: f64, longitude_false_origin: f64,
        latitude_first_parallel: f64, latitude_second_parallel: f64,
        easting_false_origin: f64, northing_false_origin: f64, ellipsoid_scaling_factor: f64,
    }
    conversion_builder! {
        /// Lambert Conic Conformal (2SP, Belgium variant).
        lambert_conic_conformal_2sp_belgium => ffi::conversion_lambert_conic_conformal_2sp_belgium;
        latitude_false_origin: f64, longitude_false_origin: f64,
        latitude_first_parallel: f64, latitude_second_parallel: f64,
        easting_false_origin: f64, northing_false_origin: f64,
    }
    conversion_builder! {
        /// Azimuthal equidistant.
        azimuthal_equidistant => ffi::conversion_azimuthal_equidistant;
        latitude_nat_origin: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Guam projection.
        guam_projection => ffi::conversion_guam_projection;
        latitude_nat_origin: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Bonne projection.
        bonne => ffi::conversion_bonne;
        latitude_nat_origin: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Lambert Cylindrical Equal Area (spherical).
        lambert_cylindrical_equal_area_spherical => ffi::conversion_lambert_cylindrical_equal_area_spherical;
        latitude_first_parallel: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Lambert Cylindrical Equal Area.
        lambert_cylindrical_equal_area => ffi::conversion_lambert_cylindrical_equal_area;
        latitude_first_parallel: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Cassini–Soldner.
        cassini_soldner => ffi::conversion_cassini_soldner;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Equidistant conic.
        equidistant_conic => ffi::conversion_equidistant_conic;
        center_lat: f64, center_long: f64,
        latitude_first_parallel: f64, latitude_second_parallel: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Eckert I.
        eckert_i => ffi::conversion_eckert_i;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Eckert II.
        eckert_ii => ffi::conversion_eckert_ii;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Eckert III.
        eckert_iii => ffi::conversion_eckert_iii;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Eckert IV.
        eckert_iv => ffi::conversion_eckert_iv;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Eckert V.
        eckert_v => ffi::conversion_eckert_v;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Eckert VI.
        eckert_vi => ffi::conversion_eckert_vi;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Equidistant cylindrical.
        equidistant_cylindrical => ffi::conversion_equidistant_cylindrical;
        latitude_first_parallel: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Equidistant cylindrical (spherical).
        equidistant_cylindrical_spherical => ffi::conversion_equidistant_cylindrical_spherical;
        latitude_first_parallel: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Gall (Gall–Peters).
        gall => ffi::conversion_gall;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Goode Homolosine.
        goode_homolosine => ffi::conversion_goode_homolosine;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Interrupted Goode Homolosine.
        interrupted_goode_homolosine => ffi::conversion_interrupted_goode_homolosine;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Geostationary Satellite, sweep X.
        geostationary_satellite_sweep_x => ffi::conversion_geostationary_satellite_sweep_x;
        center_long: f64, height: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Geostationary Satellite, sweep Y.
        geostationary_satellite_sweep_y => ffi::conversion_geostationary_satellite_sweep_y;
        center_long: f64, height: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Gnomonic.
        gnomonic => ffi::conversion_gnomonic;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Hotine Oblique Mercator (variant A).
        hotine_oblique_mercator_variant_a => ffi::conversion_hotine_oblique_mercator_variant_a;
        latitude_projection_centre: f64, longitude_projection_centre: f64,
        azimuth_initial_line: f64, angle_from_rectified_to_skrew_grid: f64,
        scale: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Hotine Oblique Mercator (variant B).
        hotine_oblique_mercator_variant_b => ffi::conversion_hotine_oblique_mercator_variant_b;
        latitude_projection_centre: f64, longitude_projection_centre: f64,
        azimuth_initial_line: f64, angle_from_rectified_to_skrew_grid: f64,
        scale: f64, easting_projection_centre: f64, northing_projection_centre: f64,
    }
    conversion_builder! {
        /// Hotine Oblique Mercator (two-point natural origin).
        hotine_oblique_mercator_two_point_natural_origin => ffi::conversion_hotine_oblique_mercator_two_point_natural_origin;
        latitude_projection_centre: f64, latitude_point1: f64, longitude_point1: f64,
        latitude_point2: f64, longitude_point2: f64, scale: f64,
        easting_projection_centre: f64, northing_projection_centre: f64,
    }
    conversion_builder! {
        /// Laborde Oblique Mercator.
        laborde_oblique_mercator => ffi::conversion_laborde_oblique_mercator;
        latitude_projection_centre: f64, longitude_projection_centre: f64,
        azimuth_initial_line: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// International Map of the World Polyconic.
        international_map_world_polyconic => ffi::conversion_international_map_world_polyconic;
        center_long: f64, latitude_first_parallel: f64, latitude_second_parallel: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Krovak (north oriented).
        krovak_north_oriented => ffi::conversion_krovak_north_oriented;
        latitude_projection_centre: f64, longitude_of_origin: f64,
        colatitude_cone_axis: f64, latitude_pseudo_standard_parallel: f64,
        scale_factor_pseudo_standard_parallel: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Krovak.
        krovak => ffi::conversion_krovak;
        latitude_projection_centre: f64, longitude_of_origin: f64,
        colatitude_cone_axis: f64, latitude_pseudo_standard_parallel: f64,
        scale_factor_pseudo_standard_parallel: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Lambert Azimuthal Equal Area.
        lambert_azimuthal_equal_area => ffi::conversion_lambert_azimuthal_equal_area;
        latitude_nat_origin: f64, longitude_nat_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Miller Cylindrical.
        miller_cylindrical => ffi::conversion_miller_cylindrical;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Mercator (variant A).
        mercator_variant_a => ffi::conversion_mercator_variant_a;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Mercator (variant B).
        mercator_variant_b => ffi::conversion_mercator_variant_b;
        latitude_first_parallel: f64, center_long: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Popular Visualisation Pseudo-Mercator.
        popular_visualisation_pseudo_mercator => ffi::conversion_popular_visualisation_pseudo_mercator;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Mollweide.
        mollweide => ffi::conversion_mollweide;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// New Zealand Mapping Grid.
        new_zealand_mapping_grid => ffi::conversion_new_zealand_mapping_grid;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Oblique Stereographic.
        oblique_stereographic => ffi::conversion_oblique_stereographic;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Orthographic.
        orthographic => ffi::conversion_orthographic;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// American Polyconic.
        american_polyconic => ffi::conversion_american_polyconic;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Polar Stereographic (variant A).
        polar_stereographic_variant_a => ffi::conversion_polar_stereographic_variant_a;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Polar Stereographic (variant B).
        polar_stereographic_variant_b => ffi::conversion_polar_stereographic_variant_b;
        latitude_standard_parallel: f64, longitude_of_origin: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Robinson.
        robinson => ffi::conversion_robinson;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Sinusoidal.
        sinusoidal => ffi::conversion_sinusoidal;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Stereographic.
        stereographic => ffi::conversion_stereographic;
        center_lat: f64, center_long: f64, scale: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Van der Grinten.
        van_der_grinten => ffi::conversion_van_der_grinten;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Wagner I.
        wagner_i => ffi::conversion_wagner_i;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Wagner II.
        wagner_ii => ffi::conversion_wagner_ii;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Wagner III.
        wagner_iii => ffi::conversion_wagner_iii;
        latitude_true_scale: f64, center_long: f64,
        false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Wagner IV.
        wagner_iv => ffi::conversion_wagner_iv;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Wagner V.
        wagner_v => ffi::conversion_wagner_v;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Wagner VI.
        wagner_vi => ffi::conversion_wagner_vi;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Wagner VII.
        wagner_vii => ffi::conversion_wagner_vii;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Quadrilateralized Spherical Cube.
        quadrilateralized_spherical_cube => ffi::conversion_quadrilateralized_spherical_cube;
        center_lat: f64, center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Spherical Cross-Track Height.
        spherical_cross_track_height => ffi::conversion_spherical_cross_track_height;
        peg_point_lat: f64, peg_point_long: f64, peg_point_heading: f64, peg_point_height: f64,
    }
    conversion_builder! {
        /// Equal Earth.
        equal_earth => ffi::conversion_equal_earth;
        center_long: f64, false_easting: f64, false_northing: f64,
    }
    conversion_builder! {
        /// Vertical Perspective.
        vertical_perspective => ffi::conversion_vertical_perspective;
        topo_origin_lat: f64, topo_origin_long: f64, topo_origin_height: f64,
        view_point_height: f64, false_easting: f64, false_northing: f64,
    }
    conversion_ang_builder! {
        /// GRIB-convention pole rotation (angular units only).
        pole_rotation_grib_convention => ffi::conversion_pole_rotation_grib_convention;
        south_pole_lat_in_unrotated_crs: f64, south_pole_long_in_unrotated_crs: f64,
        axis_rotation: f64,
    }
    conversion_ang_builder! {
        /// NetCDF-CF-convention pole rotation (angular units only).
        pole_rotation_netcdf_cf_convention => ffi::conversion_pole_rotation_netcdf_cf_convention;
        grid_north_pole_latitude: f64, grid_north_pole_longitude: f64,
        north_pole_grid_longitude: f64,
    }
}

fn coordinate_system_type(
    kind: crate::bindings::PJ_COORDINATE_SYSTEM_TYPE,
) -> CoordinateSystemType {
    match kind {
        crate::bindings::PJ_CS_TYPE_CARTESIAN => CoordinateSystemType::Cartesian,
        crate::bindings::PJ_CS_TYPE_ELLIPSOIDAL => CoordinateSystemType::Ellipsoidal,
        crate::bindings::PJ_CS_TYPE_VERTICAL => CoordinateSystemType::Vertical,
        crate::bindings::PJ_CS_TYPE_SPHERICAL => CoordinateSystemType::Spherical,
        crate::bindings::PJ_CS_TYPE_ORDINAL => CoordinateSystemType::Ordinal,
        crate::bindings::PJ_CS_TYPE_PARAMETRIC => CoordinateSystemType::Parametric,
        crate::bindings::PJ_CS_TYPE_DATETIME_TEMPORAL => CoordinateSystemType::DateTimeTemporal,
        crate::bindings::PJ_CS_TYPE_TEMPORAL_COUNT => CoordinateSystemType::TemporalCount,
        crate::bindings::PJ_CS_TYPE_TEMPORAL_MEASURE => CoordinateSystemType::TemporalMeasure,
        _ => CoordinateSystemType::Unknown,
    }
}
