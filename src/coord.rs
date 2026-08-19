//! Coordinate values and the zero-copy batch view.
//!
//! This module is the single home for all coordinate *data*:
//! - the concrete [`Coord2`], [`Coord3`], [`Coord4`] value types,
//! - the zero-allocation [`Coord`] trait (transform user types in place),
//! - the [`CoordBatch`] structure-of-arrays (SOA) view that maps directly to
//!   `proj_trans_generic` (the zero-copy batch fast path).

use crate::errors::{ProxiError, Result};

/// A coordinate that can be read component-wise as `(x, y, z, t)`.
///
/// The minimum structure required for a transform: `x` and `y` are mandatory,
/// `z`/`t` are optional (defaulting to `0.0` / `f64::INFINITY` as PROJ expects
/// for 2D). Implement it for your own types to transform them in place with no
/// intermediate allocation.
pub trait Coord: Copy {
    /// The `x` (or longitude / easting) component.
    fn x(&self) -> f64;
    /// The `y` (or latitude / northing) component.
    fn y(&self) -> f64;
    /// The `z` (height / geocentric-Z / vertical) component. Defaults to `0.0`.
    fn z(&self) -> f64 {
        0.0
    }
    /// The `t` (time) component. Defaults to `f64::INFINITY` (PROJ's "no time").
    fn t(&self) -> f64 {
        f64::INFINITY
    }
    /// Rebuild a coordinate from its four components.
    fn from_xyzt(x: f64, y: f64, z: f64, t: f64) -> Self;
}

/// A point in 2D space. The components are not CRS-specific; a `Coord2`
/// may represent longitude/latitude, easting/northing, or any other pairing.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coord2 {
    pub x: f64,
    pub y: f64,
}

impl Coord2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

impl Coord for Coord2 {
    #[inline]
    fn x(&self) -> f64 {
        self.x
    }
    #[inline]
    fn y(&self) -> f64 {
        self.y
    }
    #[inline]
    fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
        Self { x, y }
    }
}

impl From<(f64, f64)> for Coord2 {
    fn from((x, y): (f64, f64)) -> Self {
        Self { x, y }
    }
}

impl From<Coord2> for (f64, f64) {
    fn from(c: Coord2) -> Self {
        (c.x, c.y)
    }
}

/// A point in 3D space (x, y, z).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coord3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Coord3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl Coord for Coord3 {
    #[inline]
    fn x(&self) -> f64 {
        self.x
    }
    #[inline]
    fn y(&self) -> f64 {
        self.y
    }
    #[inline]
    fn z(&self) -> f64 {
        self.z
    }
    #[inline]
    fn from_xyzt(x: f64, y: f64, z: f64, _t: f64) -> Self {
        Self { x, y, z }
    }
}

impl From<(f64, f64, f64)> for Coord3 {
    fn from((x, y, z): (f64, f64, f64)) -> Self {
        Self { x, y, z }
    }
}

impl From<Coord3> for (f64, f64, f64) {
    fn from(c: Coord3) -> Self {
        (c.x, c.y, c.z)
    }
}

/// A point in 4D space (x, y, z, t). The fourth component is time.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coord4 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub t: f64,
}

impl Coord4 {
    pub const fn new(x: f64, y: f64, z: f64, t: f64) -> Self {
        Self { x, y, z, t }
    }
}

impl Coord for Coord4 {
    #[inline]
    fn x(&self) -> f64 {
        self.x
    }
    #[inline]
    fn y(&self) -> f64 {
        self.y
    }
    #[inline]
    fn z(&self) -> f64 {
        self.z
    }
    #[inline]
    fn t(&self) -> f64 {
        self.t
    }
    #[inline]
    fn from_xyzt(x: f64, y: f64, z: f64, t: f64) -> Self {
        Self { x, y, z, t }
    }
}

impl From<(f64, f64, f64, f64)> for Coord4 {
    fn from((x, y, z, t): (f64, f64, f64, f64)) -> Self {
        Self { x, y, z, t }
    }
}

impl From<Coord4> for (f64, f64, f64, f64) {
    fn from(c: Coord4) -> Self {
        (c.x, c.y, c.z, c.t)
    }
}

/// 2-D tuple (`(x, y)`).
impl Coord for (f64, f64) {
    #[inline]
    fn x(&self) -> f64 {
        self.0
    }
    #[inline]
    fn y(&self) -> f64 {
        self.1
    }
    #[inline]
    fn z(&self) -> f64 {
        0.0
    }
    #[inline]
    fn t(&self) -> f64 {
        f64::INFINITY
    }
    #[inline]
    fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
        (x, y)
    }
}

/// 3-D tuple (`(x, y, z)`).
impl Coord for (f64, f64, f64) {
    #[inline]
    fn x(&self) -> f64 {
        self.0
    }
    #[inline]
    fn y(&self) -> f64 {
        self.1
    }
    #[inline]
    fn z(&self) -> f64 {
        self.2
    }
    #[inline]
    fn t(&self) -> f64 {
        f64::INFINITY
    }
    #[inline]
    fn from_xyzt(x: f64, y: f64, z: f64, _t: f64) -> Self {
        (x, y, z)
    }
}

/// 4-D tuple (`(x, y, z, t)`).
impl Coord for (f64, f64, f64, f64) {
    #[inline]
    fn x(&self) -> f64 {
        self.0
    }
    #[inline]
    fn y(&self) -> f64 {
        self.1
    }
    #[inline]
    fn z(&self) -> f64 {
        self.2
    }
    #[inline]
    fn t(&self) -> f64 {
        self.3
    }
    #[inline]
    fn from_xyzt(x: f64, y: f64, z: f64, t: f64) -> Self {
        (x, y, z, t)
    }
}

/// The four component slices of a [`CoordBatch`], returned by
/// [`CoordBatch::into_parts`] for internal consumption at the FFI boundary.
type BatchParts<'a> = (
    &'a mut [f64],
    &'a mut [f64],
    Option<&'a mut [f64]>,
    Option<&'a mut [f64]>,
);

/// A borrowed, structure-of-arrays (SOA) batch of coordinates.
///
/// `x` and `y` are required and must be the same length; `z` and `t` are
/// optional and, when present, must match. Validation happens at construction,
/// so a [`CoordBatch`] that exists is internally consistent.
///
/// This maps directly onto `proj_trans_generic` and is the zero-copy fast path
/// for batch transforms.
pub struct CoordBatch<'a> {
    x: &'a mut [f64],
    y: &'a mut [f64],
    z: Option<&'a mut [f64]>,
    t: Option<&'a mut [f64]>,
}

impl<'a> CoordBatch<'a> {
    /// Create a 2D batch. `x` and `y` must be the same length.
    pub fn new(x: &'a mut [f64], y: &'a mut [f64]) -> Result<Self> {
        Self::validate_len("x", x.len(), y.len())?;
        Ok(Self {
            x,
            y,
            z: None,
            t: None,
        })
    }

    /// Add a `z` buffer. Must match the length of `x`.
    pub fn with_z(mut self, z: &'a mut [f64]) -> Result<Self> {
        let n = self.x.len();
        Self::validate_len("z", z.len(), n)?;
        self.z = Some(z);
        Ok(self)
    }

    /// Add a `t` buffer. Must match the length of `x`.
    pub fn with_t(mut self, t: &'a mut [f64]) -> Result<Self> {
        let n = self.x.len();
        Self::validate_len("t", t.len(), n)?;
        self.t = Some(t);
        Ok(self)
    }

    /// The number of points in this batch.
    pub fn len(&self) -> usize {
        self.x.len()
    }

    /// The `x` component buffer.
    pub fn x(&self) -> &[f64] {
        self.x
    }

    /// The `y` component buffer.
    pub fn y(&self) -> &[f64] {
        self.y
    }

    /// The optional `z` component buffer.
    pub fn z(&self) -> Option<&[f64]> {
        self.z.as_deref()
    }

    /// The optional `t` component buffer.
    pub fn t(&self) -> Option<&[f64]> {
        self.t.as_deref()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// Consume the batch and return its component slices for internal use.
    ///
    /// `pub(crate)`: external callers construct validated batches through the
    /// builders and read components through the accessors; the free slices are
    /// only needed inside the crate where the safety argument is established
    /// at the FFI boundary.
    pub(crate) fn into_parts(self) -> BatchParts<'a> {
        (self.x, self.y, self.z, self.t)
    }

    fn validate_len(name: &'static str, actual: usize, expected: usize) -> Result<()> {
        if actual != expected {
            return Err(ProxiError::LengthMismatch {
                name,
                expected,
                actual,
            });
        }
        Ok(())
    }
}

// These bridge the *input/output* coordinate types of popular geospatial /
// math crates onto the existing `Coord` trait — the transforms themselves
// (Transformer::transform_coord / transform_coords) are unchanged and generic
// over `Coord`. No core abstraction is touched.

#[cfg(feature = "geo")]
mod geo_adapter {
    use super::Coord;

    impl Coord for geo::Coord {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
            Self { x, y }
        }
    }

    impl Coord for geo::Point {
        #[inline]
        fn x(&self) -> f64 {
            self.0.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.0.y
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
            Self::new(x, y)
        }
    }
}

#[cfg(feature = "nalgebra")]
mod nalgebra_adapter {
    use super::Coord;
    use nalgebra::{Point2, Point3, Vector2, Vector3};

    impl Coord for Vector2<f64> {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
            Self::new(x, y)
        }
    }

    impl Coord for Vector3<f64> {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn z(&self) -> f64 {
            self.z
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, z: f64, _t: f64) -> Self {
            Self::new(x, y, z)
        }
    }

    impl Coord for Point2<f64> {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
            Self::new(x, y)
        }
    }

    impl Coord for Point3<f64> {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn z(&self) -> f64 {
            self.z
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, z: f64, _t: f64) -> Self {
            Self::new(x, y, z)
        }
    }
}

#[cfg(feature = "glam")]
mod glam_adapter {
    use super::Coord;
    use glam::{DVec2, DVec3, DVec4};

    impl Coord for DVec2 {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, _z: f64, _t: f64) -> Self {
            Self::new(x, y)
        }
    }

    impl Coord for DVec3 {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn z(&self) -> f64 {
            self.z
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, z: f64, _t: f64) -> Self {
            Self::new(x, y, z)
        }
    }

    impl Coord for DVec4 {
        #[inline]
        fn x(&self) -> f64 {
            self.x
        }
        #[inline]
        fn y(&self) -> f64 {
            self.y
        }
        #[inline]
        fn z(&self) -> f64 {
            self.z
        }
        #[inline]
        fn t(&self) -> f64 {
            self.w
        }
        #[inline]
        fn from_xyzt(x: f64, y: f64, z: f64, t: f64) -> Self {
            Self::new(x, y, z, t)
        }
    }
}
