//! Ellipsoidal geodesic calculations, implemented in Rust over `proxi::sys`.
//!
//! PROJ's `geodesic.h` exposes the `geod_geodesic` / `geod_geodesicline` /
//! `geod_polygon` structs and the `geod_*` functions. Because `proxi::sys`
//! binds them concretely (they are plain data), we hold the state by value and
//! call `sys::geod_*` directly — no C shim, no opaque handle, no manual free.

use crate::context::Context;
use crate::errors::{ProxiError, Result};
use crate::sys;
use std::marker::PhantomData;
use std::mem::MaybeUninit;

/// Result of an inverse geodesic calculation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeodesicInverse {
    /// Distance between the points in metres.
    pub distance_meters: f64,
    /// Forward azimuth at the first point, in degrees.
    pub forward_azimuth_degree: f64,
    /// Reverse azimuth at the second point, in degrees.
    pub reverse_azimuth_degree: f64,
}

/// Extended inverse result (the `geninverse` output).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeodesicInverseIndexed {
    pub distance_meters: f64,
    pub forward_azimuth_degree: f64,
    pub reverse_azimuth_degree: f64,
    /// Reduced length, metres.
    pub m12_meters: f64,
    /// Geodesic scale at point 1 (unitless).
    pub m12_scale: f64,
    /// Geodesic scale at point 2 (unitless).
    pub m21_scale: f64,
    /// Geodesic area enclosed by the geodesic between the two points, m².
    pub area_square_meters: f64,
    /// Angle subtended at the centre of the ellipsoid (degrees or as given).
    pub a12: f64,
}

/// Result of a direct geodesic calculation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeodesicDirect {
    pub longitude_degree: f64,
    pub latitude_degree: f64,
    pub reverse_azimuth_degree: f64,
}

/// Extended direct result (the `gendirect` output).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeodesicDirectIndexed {
    pub longitude_degree: f64,
    pub latitude_degree: f64,
    pub reverse_azimuth_degree: f64,
    /// Distance travelled, metres.
    pub distance_meters: f64,
    /// Reduced length, metres.
    pub m12_meters: f64,
    /// Geodesic scale at point 1.
    pub m12_scale: f64,
    /// Geodesic scale at point 2.
    pub m21_scale: f64,
    /// Geodesic area, m².
    pub area_square_meters: f64,
}

/// Result of a polygon geodesic calculation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeodesicPolygon {
    pub area_square_meters: f64,
    pub perimeter_meters: f64,
}

/// A reusable ellipsoidal geodesic calculator.
///
/// The object is thread-bound. Construct one per worker and reuse it for
/// scalar or batch calculations. The ellipsoid state is a plain PROJ
/// `geod_geodesic` held inline (no heap, no allocation, no manual free).
pub struct Geod<'context> {
    /// The PROJ ellipsoid state, held inline (plain data; no allocation).
    g: sys::geod_geodesic,
    _context: PhantomData<&'context Context>,
    _not_send_sync: PhantomData<std::rc::Rc<()>>,
}

/// A precomputed geodesic line, supporting repeated position queries
/// (streaming) and optional arc mode / longitude unrolling.
///
/// Holds the PROJ `geod_geodesicline` by value; `caps` records which outputs
/// are valid based on the capabilities requested at construction.
pub struct Line {
    l: sys::geod_geodesicline,
    caps: u32,
}

/// Result of a line position query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinePosition {
    pub longitude_degree: f64,
    pub latitude_degree: f64,
    pub reverse_azimuth_degree: f64,
    pub distance_meters: f64,
    pub m12_meters: f64,
    pub m12_scale: f64,
    pub m21_scale: f64,
    pub area_square_meters: f64,
    /// Arc angle (degrees), always returned.
    pub a12: f64,
}

/// Streaming polygon/geodesic-area accumulator (uses `geod_polygon`).
pub struct PolygonBuilder {
    poly: sys::geod_polygon,
}

impl<'context> Geod<'context> {
    /// Construct a WGS84 geodesic calculator.
    pub fn wgs84(context: &'context Context) -> Result<Self> {
        Self::from_ellipsoid(context, 6_378_137.0, 298.257_223_563)
    }

    /// Construct a geodesic from semi-major axis and inverse flattening.
    pub fn from_ellipsoid(
        _context: &'context Context,
        semi_major_meters: f64,
        inverse_flattening: f64,
    ) -> Result<Self> {
        if !semi_major_meters.is_finite()
            || semi_major_meters <= 0.0
            || !inverse_flattening.is_finite()
            || inverse_flattening <= 0.0
        {
            return Err(ProxiError::InvalidCrs {
                input: "ellipsoid".to_string(),
                message: "semi-major axis and inverse flattening must be finite and positive"
                    .to_string(),
            });
        }
        let flattening = 1.0 / inverse_flattening;
        // SAFETY: `geod_geodesic` is a plain-data C struct; zero-init then let
        // `geod_init` populate it.
        let mut g: sys::geod_geodesic = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe { sys::geod_init(&mut g, semi_major_meters, flattening) };
        Ok(Self {
            g,
            _context: PhantomData,
            _not_send_sync: PhantomData,
        })
    }

    /// Construct a geodesic from a PROJ definition containing an ellipsoid.
    ///
    /// The ellipsoid parameters are read from the resolved PROJ object via
    /// `proj_get_ellipsoid` + `proj_ellipsoid_get_parameters`, not parsed from
    /// the string. This gives the *effective* ellipsoid PROJ selected (which
    /// may differ from the text for e.g. aliases) and fails clearly if the
    /// definition has no resolvable ellipsoid — no silent WGS84 fallback.
    pub fn from_proj_string(context: &'context Context, definition: &str) -> Result<Self> {
        // Create + validate the object (must be a CRS with an ellipsoid).
        let obj = crate::ffi::create(context, definition)?;
        let (semi_major, semi_minor, inverse_flattening, _computed) =
            crate::ffi::crs_ellipsoid(&obj, context).ok_or_else(|| ProxiError::InvalidCrs {
                input: definition.to_string(),
                message: "definition has no resolvable ellipsoid".to_string(),
            })?;
        // `proj_ellipsoid_get_parameters` returns the inverse flattening, which
        // is what `from_ellipsoid` expects. It may be -1 when the ellipsoid is
        // a sphere or when it was derived from semi-minor; handle that.
        if inverse_flattening > 0.0 {
            Self::from_ellipsoid(context, semi_major, inverse_flattening)
        } else {
            let flattening = if semi_major > 0.0 && semi_minor > 0.0 {
                (semi_major - semi_minor) / semi_major
            } else {
                return Err(ProxiError::InvalidCrs {
                    input: definition.to_string(),
                    message: "invalid semi-major axis".to_string(),
                });
            };
            // geod_init accepts a flattening; 0 == sphere.
            let mut g: sys::geod_geodesic = unsafe { MaybeUninit::zeroed().assume_init() };
            unsafe { sys::geod_init(&mut g, semi_major, flattening) };
            Ok(Self {
                g,
                _context: PhantomData,
                _not_send_sync: PhantomData,
            })
        }
    }

    /// Solve the inverse geodesic problem for two longitude/latitude points.
    /// Inputs and azimuth outputs are degrees; distance is metres.
    pub fn inverse(
        &self,
        first_longitude_degree: f64,
        first_latitude_degree: f64,
        second_longitude_degree: f64,
        second_latitude_degree: f64,
    ) -> Result<GeodesicInverse> {
        let mut s12 = 0.0;
        let mut azi1 = 0.0;
        let mut azi2 = 0.0;
        // SAFETY: output pointers are valid for the call; the geodesic state
        // is live.
        unsafe {
            sys::geod_inverse(
                &self.g as *const _,
                first_latitude_degree,
                first_longitude_degree,
                second_latitude_degree,
                second_longitude_degree,
                &mut s12,
                &mut azi1,
                &mut azi2,
            );
        }
        Ok(GeodesicInverse {
            distance_meters: s12,
            forward_azimuth_degree: azi1,
            reverse_azimuth_degree: azi2,
        })
    }

    /// Extended inverse: additionally returns reduced length, geodesic scales,
    /// area and the central angle `a12` (via `geod_geninverse`).
    pub fn inverse_indexed(
        &self,
        first_longitude_degree: f64,
        first_latitude_degree: f64,
        second_longitude_degree: f64,
        second_latitude_degree: f64,
    ) -> Result<GeodesicInverseIndexed> {
        let mut s12 = 0.0;
        let mut azi1 = 0.0;
        let mut azi2 = 0.0;
        let mut m12 = 0.0;
        let mut m12s = 0.0;
        let mut m21s = 0.0;
        let mut s12area = 0.0;
        // SAFETY: output pointers are valid for the call; `a12` is the return.
        let a12 = unsafe {
            sys::geod_geninverse(
                &self.g as *const _,
                first_latitude_degree,
                first_longitude_degree,
                second_latitude_degree,
                second_longitude_degree,
                &mut s12,
                &mut azi1,
                &mut azi2,
                &mut m12,
                &mut m12s,
                &mut m21s,
                &mut s12area,
            )
        };
        Ok(GeodesicInverseIndexed {
            distance_meters: s12,
            forward_azimuth_degree: azi1,
            reverse_azimuth_degree: azi2,
            m12_meters: m12,
            m12_scale: m12s,
            m21_scale: m21s,
            area_square_meters: s12area,
            a12,
        })
    }

    /// Solve the direct geodesic problem. Inputs and outputs are degrees;
    /// distance is metres.
    pub fn direct(
        &self,
        longitude_degree: f64,
        latitude_degree: f64,
        azimuth_degree: f64,
        distance_meters: f64,
    ) -> Result<GeodesicDirect> {
        let mut lat2 = 0.0;
        let mut lon2 = 0.0;
        let mut azi2 = 0.0;
        // SAFETY: output pointers are valid for the call.
        unsafe {
            sys::geod_direct(
                &self.g as *const _,
                latitude_degree,
                longitude_degree,
                azimuth_degree,
                distance_meters,
                &mut lat2,
                &mut lon2,
                &mut azi2,
            );
        }
        Ok(GeodesicDirect {
            longitude_degree: lon2,
            latitude_degree: lat2,
            reverse_azimuth_degree: azi2,
        })
    }

    /// Extended direct: additionally returns distance, reduced length,
    /// geodesic scales and area (via `geod_gendirect`). `arcmode` interprets
    /// `distance_or_arc_degree` as an arc angle in degrees instead of metres;
    /// `longitude_unroll` keeps longitudes monotonic along the geodesic.
    pub fn direct_indexed(
        &self,
        longitude_degree: f64,
        latitude_degree: f64,
        azimuth_degree: f64,
        distance_or_arc_degree: f64,
        arcmode: bool,
        longitude_unroll: bool,
    ) -> Result<GeodesicDirectIndexed> {
        let mut flags = 0u32;
        if arcmode {
            flags |= sys::geod_flags_GEOD_ARCMODE as u32;
        }
        if longitude_unroll {
            flags |= sys::geod_flags_GEOD_LONG_UNROLL as u32;
        }
        let mut lat2 = 0.0;
        let mut lon2 = 0.0;
        let mut azi2 = 0.0;
        let mut s12 = 0.0;
        let mut m12 = 0.0;
        let mut m12s = 0.0;
        let mut m21s = 0.0;
        let mut s12area = 0.0;
        // SAFETY: output pointers are valid for the call.
        unsafe {
            sys::geod_gendirect(
                &self.g as *const _,
                latitude_degree,
                longitude_degree,
                azimuth_degree,
                flags,
                distance_or_arc_degree,
                &mut lat2,
                &mut lon2,
                &mut azi2,
                &mut s12,
                &mut m12,
                &mut m12s,
                &mut m21s,
                &mut s12area,
            );
        }
        Ok(GeodesicDirectIndexed {
            longitude_degree: lon2,
            latitude_degree: lat2,
            reverse_azimuth_degree: azi2,
            distance_meters: s12,
            m12_meters: m12,
            m12_scale: m12s,
            m21_scale: m21s,
            area_square_meters: s12area,
        })
    }

    /// Build a [`Line`] from an initial point, azimuth and distance (or arc).
    ///
    /// `arcmode` interprets `distance_or_arc_degree` as degrees (arc) instead
    /// of metres. `caps` requests which outputs are valid on `Line::position`
    /// (see [`LineCaps`]). `longitude_unroll` records that positions should
    /// unroll longitude (monotonic along the geodesic).
    #[allow(clippy::too_many_arguments)]
    pub fn line(
        &self,
        longitude_degree: f64,
        latitude_degree: f64,
        azimuth_degree: f64,
        distance_or_arc_degree: f64,
        arcmode: bool,
        longitude_unroll: bool,
        caps: LineCaps,
    ) -> Line {
        let mut flags = 0u32;
        if arcmode {
            flags |= sys::geod_flags_GEOD_ARCMODE as u32;
        }
        if longitude_unroll {
            flags |= sys::geod_flags_GEOD_LONG_UNROLL as u32;
        }
        // SAFETY: `geod_geodesicline` is plain data; `geod_gendirectline` fills it.
        let mut l: sys::geod_geodesicline = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe {
            sys::geod_gendirectline(
                &mut l,
                &self.g as *const _,
                latitude_degree,
                longitude_degree,
                azimuth_degree,
                flags,
                distance_or_arc_degree,
                caps.bits(),
            );
        }
        Line {
            l,
            caps: caps.bits(),
        }
    }

    /// Build a [`Line`] from two endpoints (inverse line).
    pub fn line_between(
        &self,
        first_longitude_degree: f64,
        first_latitude_degree: f64,
        second_longitude_degree: f64,
        second_latitude_degree: f64,
        caps: LineCaps,
    ) -> Line {
        // SAFETY: `geod_geodesicline` is plain data; `geod_inverseline` fills it.
        let mut l: sys::geod_geodesicline = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe {
            sys::geod_inverseline(
                &mut l,
                &self.g as *const _,
                first_latitude_degree,
                first_longitude_degree,
                second_latitude_degree,
                second_longitude_degree,
                caps.bits(),
            );
        }
        Line {
            l,
            caps: caps.bits(),
        }
    }

    /// Compute signed geodesic area and perimeter for a closed polygon.
    /// The polygon is implicitly closed; input slices contain vertices in
    /// longitude/latitude order by parallel index.
    pub fn polygon_area_perimeter(
        &self,
        longitudes_degree: &[f64],
        latitudes_degree: &[f64],
    ) -> Result<GeodesicPolygon> {
        if longitudes_degree.len() != latitudes_degree.len() {
            return Err(ProxiError::LengthMismatch {
                name: "latitudes",
                expected: longitudes_degree.len(),
                actual: latitudes_degree.len(),
            });
        }
        // SAFETY: `geod_polygon` is a plain-data C struct initialized by
        // `geod_polygon_init`.
        let mut poly: sys::geod_polygon = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe { sys::geod_polygon_init(&mut poly, 0) };
        for (lon, lat) in longitudes_degree.iter().zip(latitudes_degree) {
            // SAFETY: `poly` is live and `&self.g` matches the polygon's ellipsoid.
            unsafe { sys::geod_polygon_addpoint(&self.g as *const _, &mut poly, *lat, *lon) };
        }
        let mut area = 0.0;
        let mut perimeter = 0.0;
        // SAFETY: output pointers are valid; `poly` is fully populated.
        unsafe {
            sys::geod_polygon_compute(
                &self.g as *const _,
                &poly as *const _,
                0,
                1,
                &mut area,
                &mut perimeter,
            );
        }
        Ok(GeodesicPolygon {
            area_square_meters: area,
            perimeter_meters: perimeter,
        })
    }

    /// Test whether a point is inside/outside the polygon built by
    /// [`Geod::polygon_builder`], extending the current polygon state.
    pub fn polygon_test_point(
        &self,
        polygon: &mut PolygonBuilder,
        longitude_degree: f64,
        latitude_degree: f64,
    ) -> Result<f64> {
        let mut out = 0.0;
        let mut _perimeter = 0.0;
        let _num = unsafe {
            sys::geod_polygon_testpoint(
                &self.g as *const _,
                &polygon.poly as *const _,
                latitude_degree,
                longitude_degree,
                0,
                1,
                &mut out,
                &mut _perimeter,
            )
        };
        Ok(out)
    }

    /// Test whether an edge from the current polygon state contains the point
    /// reached by `azimuth_degree`/`distance_meters`.
    pub fn polygon_test_edge(
        &self,
        polygon: &mut PolygonBuilder,
        azimuth_degree: f64,
        distance_meters: f64,
    ) -> Result<f64> {
        let mut out = 0.0;
        let mut _perimeter = 0.0;
        let _num = unsafe {
            sys::geod_polygon_testedge(
                &self.g as *const _,
                &polygon.poly as *const _,
                azimuth_degree,
                distance_meters,
                0,
                1,
                &mut out,
                &mut _perimeter,
            )
        };
        Ok(out)
    }

    /// Write `longitudes_degree.len()` intermediate points between two points.
    /// Endpoints are excluded, matching pyproj's `Geod.npts` behavior.
    pub fn npts_into(
        &self,
        first_longitude_degree: f64,
        first_latitude_degree: f64,
        second_longitude_degree: f64,
        second_latitude_degree: f64,
        longitudes_degree: &mut [f64],
        latitudes_degree: &mut [f64],
    ) -> Result<()> {
        if longitudes_degree.len() != latitudes_degree.len() {
            return Err(ProxiError::LengthMismatch {
                name: "latitudes",
                expected: longitudes_degree.len(),
                actual: latitudes_degree.len(),
            });
        }
        if longitudes_degree.is_empty() {
            return Ok(());
        }
        let inverse = self.inverse(
            first_longitude_degree,
            first_latitude_degree,
            second_longitude_degree,
            second_latitude_degree,
        )?;
        // SAFETY: `geod_geodesicline` is plain data; `geod_inverseline` fills it.
        let mut line: sys::geod_geodesicline = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe {
            sys::geod_inverseline(
                &mut line,
                &self.g as *const _,
                first_latitude_degree,
                first_longitude_degree,
                second_latitude_degree,
                second_longitude_degree,
                0,
            );
        }
        let denominator = (longitudes_degree.len() + 1) as f64;
        for index in 0..longitudes_degree.len() {
            let distance = inverse.distance_meters * (index + 1) as f64 / denominator;
            let mut lat = 0.0;
            let mut lon = 0.0;
            let mut azi = 0.0;
            // SAFETY: output pointers are valid for the call.
            unsafe {
                sys::geod_position(&line as *const _, distance, &mut lat, &mut lon, &mut azi)
            };
            let _ = azi;
            longitudes_degree[index] = lon;
            latitudes_degree[index] = lat;
        }
        Ok(())
    }

    /// Solve inverse geodesics for parallel borrowed slices without allocation.
    #[allow(clippy::too_many_arguments)]
    pub fn inverse_batch_into(
        &self,
        first_longitudes_degree: &[f64],
        first_latitudes_degree: &[f64],
        second_longitudes_degree: &[f64],
        second_latitudes_degree: &[f64],
        distances_meters: &mut [f64],
        forward_azimuths_degree: &mut [f64],
        reverse_azimuths_degree: &mut [f64],
    ) -> Result<()> {
        let n = first_longitudes_degree.len();
        for (name, actual) in [
            ("first_latitudes", first_latitudes_degree.len()),
            ("second_longitudes", second_longitudes_degree.len()),
            ("second_latitudes", second_latitudes_degree.len()),
            ("distances", distances_meters.len()),
            ("forward_azimuths", forward_azimuths_degree.len()),
            ("reverse_azimuths", reverse_azimuths_degree.len()),
        ] {
            if actual != n {
                return Err(ProxiError::LengthMismatch {
                    name,
                    expected: n,
                    actual,
                });
            }
        }
        for index in 0..n {
            let result = self.inverse(
                first_longitudes_degree[index],
                first_latitudes_degree[index],
                second_longitudes_degree[index],
                second_latitudes_degree[index],
            )?;
            distances_meters[index] = result.distance_meters;
            forward_azimuths_degree[index] = result.forward_azimuth_degree;
            reverse_azimuths_degree[index] = result.reverse_azimuth_degree;
        }
        Ok(())
    }

    /// Open a streaming polygon accumulator over this geodesic.
    ///
    /// Add vertices with [`PolygonBuilder::add_point`] / [`PolygonBuilder::add_edge`]
    /// and finalize with [`PolygonBuilder::compute`].
    pub fn polygon_builder(&self) -> PolygonBuilder {
        // SAFETY: `geod_polygon` is plain data initialized by `geod_polygon_init`.
        let mut poly: sys::geod_polygon = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe { sys::geod_polygon_init(&mut poly, 0) };
        PolygonBuilder { poly }
    }

    /// Open a streaming polyline-accumulator (perimeter only, no area).
    pub fn polyline_builder(&self) -> PolygonBuilder {
        // SAFETY: `geod_polygon` is plain data initialized by `geod_polygon_init`
        // with polylinep=1 (polyline, no area).
        let mut poly: sys::geod_polygon = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe { sys::geod_polygon_init(&mut poly, 1) };
        PolygonBuilder { poly }
    }

    /// The underlying PROJ geodesic state (public so power users can pass it to
    /// `proxi::sys::geod_*` directly).
    pub fn as_ptr(&self) -> *const sys::geod_geodesic {
        &self.g as *const _
    }
}

/// Capabilities requested when building a [`Line`] (which outputs are valid).
///
/// Mirrors the C `geod_mask` bits. Use bitwise-or to combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineCaps(u32);

impl LineCaps {
    pub const LATITUDE: Self = Self(sys::geod_mask_GEOD_LATITUDE as u32);
    pub const LONGITUDE: Self = Self(sys::geod_mask_GEOD_LONGITUDE as u32);
    pub const AZIMUTH: Self = Self(sys::geod_mask_GEOD_AZIMUTH as u32);
    pub const DISTANCE: Self = Self(sys::geod_mask_GEOD_DISTANCE as u32);
    pub const DISTANCE_IN: Self = Self(sys::geod_mask_GEOD_DISTANCE_IN as u32);
    pub const REDUCEDLENGTH: Self = Self(sys::geod_mask_GEOD_REDUCEDLENGTH as u32);
    pub const GEODESICSCALE: Self = Self(sys::geod_mask_GEOD_GEODESICSCALE as u32);
    pub const AREA: Self = Self(sys::geod_mask_GEOD_AREA as u32);
    pub const ALL: Self = Self(sys::geod_mask_GEOD_ALL as u32);

    fn bits(&self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for LineCaps {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl Line {
    /// Query the line position at a distance (metres, or arc degrees if the
    /// line was built with `arcmode`). Returns all outputs requested by the
    /// line's capability mask; outputs not requested are `0.0`.
    ///
    /// If the line was built with `longitude_unroll`, longitudes are monotonic
    /// along the line.
    pub fn position(&self, s12_or_a12: f64) -> LinePosition {
        let mut lat2 = 0.0;
        let mut lon2 = 0.0;
        let mut azi2 = 0.0;
        let mut s12 = 0.0;
        let mut m12 = 0.0;
        let mut m12s = 0.0;
        let mut m21s = 0.0;
        let mut s12area = 0.0;
        // SAFETY: output pointers are valid for the call.
        let a12 = unsafe {
            sys::geod_genposition(
                &self.l as *const _,
                0, // no flags: distance mode (unroll state is in the line)
                s12_or_a12,
                &mut lat2,
                &mut lon2,
                &mut azi2,
                &mut s12,
                &mut m12,
                &mut m12s,
                &mut m21s,
                &mut s12area,
            )
        };
        LinePosition {
            longitude_degree: lon2,
            latitude_degree: lat2,
            reverse_azimuth_degree: azi2,
            distance_meters: s12,
            m12_meters: m12,
            m12_scale: m12s,
            m21_scale: m21s,
            area_square_meters: s12area,
            a12,
        }
    }

    /// Query the line position with explicit arc-mode / longitude-unroll flags,
    /// overriding the line's construction defaults. `arcmode` interprets
    /// `s12_or_a12` as an arc angle (degrees); `longitude_unroll` enables
    /// monotonic longitude output.
    pub fn gen_position(
        &self,
        s12_or_a12: f64,
        arcmode: bool,
        longitude_unroll: bool,
    ) -> LinePosition {
        let mut flags = 0u32;
        if arcmode {
            flags |= sys::geod_flags_GEOD_ARCMODE as u32;
        }
        if longitude_unroll {
            flags |= sys::geod_flags_GEOD_LONG_UNROLL as u32;
        }
        let mut lat2 = 0.0;
        let mut lon2 = 0.0;
        let mut azi2 = 0.0;
        let mut s12 = 0.0;
        let mut m12 = 0.0;
        let mut m12s = 0.0;
        let mut m21s = 0.0;
        let mut s12area = 0.0;
        // SAFETY: output pointers are valid for the call.
        let a12 = unsafe {
            sys::geod_genposition(
                &self.l as *const _,
                flags,
                s12_or_a12,
                &mut lat2,
                &mut lon2,
                &mut azi2,
                &mut s12,
                &mut m12,
                &mut m12s,
                &mut m21s,
                &mut s12area,
            )
        };
        LinePosition {
            longitude_degree: lon2,
            latitude_degree: lat2,
            reverse_azimuth_degree: azi2,
            distance_meters: s12,
            m12_meters: m12,
            m12_scale: m12s,
            m21_scale: m21s,
            area_square_meters: s12area,
            a12,
        }
    }

    /// Change (or set) the endpoint distance, keeping the initial point and
    /// azimuth. Subsequent `position` queries measure from this new base.
    pub fn set_distance(&mut self, s13_meters: f64) {
        // SAFETY: the line is live and owned by this `Line`.
        unsafe { sys::geod_setdistance(&mut self.l, s13_meters) };
    }

    /// The capabilities this line was built with (see [`LineCaps`]).
    pub fn caps(&self) -> LineCaps {
        LineCaps(self.caps)
    }
}

impl PolygonBuilder {
    /// Add a vertex by longitude/latitude (degrees).
    pub fn add_point(&mut self, geod: &Geod, longitude_degree: f64, latitude_degree: f64) {
        // SAFETY: `poly` is live and `geod` matches its ellipsoid.
        unsafe {
            sys::geod_polygon_addpoint(
                &geod.g as *const _,
                &mut self.poly,
                latitude_degree,
                longitude_degree,
            )
        };
    }

    /// Add an edge from the previous point by azimuth (degrees) and distance (m).
    pub fn add_edge(&mut self, geod: &Geod, azimuth_degree: f64, distance_meters: f64) {
        // SAFETY: `poly` is live and `geod` matches its ellipsoid.
        unsafe {
            sys::geod_polygon_addedge(
                &geod.g as *const _,
                &mut self.poly,
                azimuth_degree,
                distance_meters,
            )
        };
    }

    /// Finalize and return the signed area (m²) and perimeter (m).
    pub fn compute(&self, geod: &Geod) -> GeodesicPolygon {
        let mut area = 0.0;
        let mut perimeter = 0.0;
        // SAFETY: output pointers are valid; `poly` is fully populated.
        unsafe {
            sys::geod_polygon_compute(
                &geod.g as *const _,
                &self.poly as *const _,
                0,
                1,
                &mut area,
                &mut perimeter,
            );
        }
        GeodesicPolygon {
            area_square_meters: area,
            perimeter_meters: perimeter,
        }
    }

    /// Clear all accumulated vertices/edges, keeping the builder reusable.
    pub fn clear(&mut self) {
        // SAFETY: `poly` is live and owned.
        unsafe { sys::geod_polygon_clear(&mut self.poly) };
    }
}
