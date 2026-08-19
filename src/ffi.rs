//! Unsafe-free facade over the raw PROJ bindings.
//!
//! All `unsafe` calls into `bindings` live in this module. The rest of
//! `proxi` operates on safe types:
//! - [`Context`] — owned `PJ_CONTEXT*`.
//! - [`ProjObj`] — owned `PJ*` bound to a context.

use crate::bindings;
use crate::context::Context;
use crate::errors::{ProxiError, Result};
use std::ffi::CString;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Owned PROJ object (`PJ*`) bound to a specific context, thread-affine.
pub(crate) struct ProjObj {
    pub(crate) raw: NonNull<bindings::PJ>,
    pub(crate) context: NonNull<bindings::PJ_CONTEXT>,
    _not_send_sync: PhantomData<std::rc::Rc<()>>,
}

impl ProjObj {
    fn new(pj: *mut bindings::PJ, ctx: NonNull<bindings::PJ_CONTEXT>) -> Option<Self> {
        NonNull::new(pj).map(|raw| Self {
            raw,
            context: ctx,
            _not_send_sync: PhantomData,
        })
    }

    pub(crate) fn as_ptr(&self) -> *mut bindings::PJ {
        self.raw.as_ptr()
    }

    pub(crate) fn context_ptr(&self) -> *mut bindings::PJ_CONTEXT {
        self.context.as_ptr()
    }
}

impl Drop for ProjObj {
    fn drop(&mut self) {
        // SAFETY: `raw` is an owned `PJ*`; PROJ takes ownership and frees it.
        unsafe { bindings::proj_destroy(self.raw.as_ptr()) };
    }
}

/// Create a `PJ*` from a PROJ definition string (e.g. EPSG code, WKT,
/// PROJ string). The object must be a CRS; callers should verify with
/// [`create_crs`].
pub(crate) fn create(ctx: &Context, definition: &str) -> Result<ProjObj> {
    let c = CString::new(definition.as_bytes()).map_err(ProxiError::from)?;
    // SAFETY: `c` is a valid NUL-terminated C string; PROJ parses it and
    // returns a new object or null.
    let pj = unsafe { bindings::proj_create(ctx.as_ptr(), c.as_ptr()) };
    ProjObj::new(pj, NonNull::new(ctx.as_ptr()).expect("live context")).ok_or_else(|| {
        let (_, msg) = ctx.errno_message();
        ProxiError::InvalidCrs {
            input: definition.to_string(),
            message: msg,
        }
    })
}

/// Create a `Crs` (verify with `proj_is_crs`).
pub(crate) fn create_crs(ctx: &Context, definition: &str) -> Result<ProjObj> {
    let obj = create(ctx, definition)?;
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    let is_crs = unsafe { bindings::proj_is_crs(obj.as_ptr()) };
    if is_crs == 0 {
        return Err(ProxiError::InvalidCrs {
            input: definition.to_string(),
            message: "definition is not a CRS".to_string(),
        });
    }
    Ok(obj)
}

/// Get a CRS's geodetic CRS (the geodetic component of a projected/geographic
/// CRS), or `None` if the object doesn't have one. Returns an owned `ProjObj`.
pub(crate) fn crs_geodetic_crs(ctx: &Context, obj: &ProjObj) -> Option<ProjObj> {
    // SAFETY: `proj_crs_get_geodetic_crs` returns an owned PJ or null.
    let raw = unsafe { bindings::proj_crs_get_geodetic_crs(ctx.as_ptr(), obj.as_ptr()) };
    ProjObj::new(raw, NonNull::new(ctx.as_ptr()).expect("live context"))
}

/// Get a CRS's horizontal datum, or `None` if absent. Returns an owned `ProjObj`.
pub(crate) fn crs_horizontal_datum(ctx: &Context, obj: &ProjObj) -> Option<ProjObj> {
    // SAFETY: `proj_crs_get_horizontal_datum` returns an owned PJ or null.
    let raw = unsafe { bindings::proj_crs_get_horizontal_datum(ctx.as_ptr(), obj.as_ptr()) };
    ProjObj::new(raw, NonNull::new(ctx.as_ptr()).expect("live context"))
}

/// Get one sub-CRS of a compound CRS by index, or `None`. Returns an owned `ProjObj`.
pub(crate) fn crs_sub_crs(ctx: &Context, obj: &ProjObj, index: usize) -> Option<ProjObj> {
    // SAFETY: `proj_crs_get_sub_crs` returns an owned PJ or null.
    let raw = unsafe { bindings::proj_crs_get_sub_crs(ctx.as_ptr(), obj.as_ptr(), index as i32) };
    ProjObj::new(raw, NonNull::new(ctx.as_ptr()).expect("live context"))
}

/// Get the coordinate operation attached to a (bound/derived) CRS, or `None`.
/// Returns an owned `ProjObj`.
pub(crate) fn crs_coord_operation(ctx: &Context, obj: &ProjObj) -> Option<ProjObj> {
    // SAFETY: `proj_crs_get_coordoperation` returns an owned PJ or null.
    let raw = unsafe { bindings::proj_crs_get_coordoperation(ctx.as_ptr(), obj.as_ptr()) };
    ProjObj::new(raw, NonNull::new(ctx.as_ptr()).expect("live context"))
}

/// Get a CRS's datum (the datum of a geodetic/vertical CRS), or `None`.
/// Returns an owned `ProjObj`.
pub(crate) fn crs_datum(ctx: &Context, obj: &ProjObj) -> Option<ProjObj> {
    // SAFETY: `proj_crs_get_datum` returns an owned PJ or null.
    let raw = unsafe { bindings::proj_crs_get_datum(ctx.as_ptr(), obj.as_ptr()) };
    ProjObj::new(raw, NonNull::new(ctx.as_ptr()).expect("live context"))
}

/// Create a CRS (or coordinate-op) object by name / identifier, via
/// `proj_create_from_name`. Returns the first matching owned object, or `None`.
///
/// The `types` slice filters the object kinds to search (e.g. projected CRSs).
/// Pass `&[]` to search all object types. `approximate` enables fuzzy matching.
pub(crate) fn create_from_name(
    ctx: &Context,
    name: &str,
    types: &[bindings::PJ_TYPE],
    approximate: bool,
) -> Option<ProjObj> {
    let c_name = CString::new(name.as_bytes()).ok()?;
    // SAFETY: `proj_create_from_name` returns a PROJ-owned list (or null).
    let list = unsafe {
        bindings::proj_create_from_name(
            ctx.as_ptr(),
            std::ptr::null(),
            c_name.as_ptr(),
            types.as_ptr(),
            types.len(),
            approximate as i32,
            1, // limitResultCount
            std::ptr::null(),
        )
    };
    let list = NonNull::new(list)?;
    // SAFETY: `proj_list_get` returns an owned PJ (valid index 0).
    let first = unsafe { bindings::proj_list_get(ctx.as_ptr(), list.as_ptr(), 0) };
    // SAFETY: the list is no longer needed after the object is copied out.
    unsafe { bindings::proj_list_destroy(list.as_ptr()) };
    ProjObj::new(first, NonNull::new(ctx.as_ptr()).expect("live context"))
}

pub(crate) fn create_crs_from_database(
    ctx: &Context,
    authority: &str,
    code: &str,
) -> Result<ProjObj> {
    let authority = CString::new(authority.as_bytes()).map_err(ProxiError::from)?;
    let code = CString::new(code.as_bytes()).map_err(ProxiError::from)?;
    // SAFETY: strings are valid for the call; PROJ returns an owned CRS.
    let obj = unsafe {
        bindings::proj_create_from_database(
            ctx.as_ptr(),
            authority.as_ptr(),
            code.as_ptr(),
            bindings::PJ_CATEGORY_CRS,
            0,
            std::ptr::null(),
        )
    };
    ProjObj::new(obj, NonNull::new(ctx.as_ptr()).expect("live context")).ok_or_else(|| {
        let (_, message) = ctx.errno_message();
        ProxiError::InvalidCrs {
            input: format!("{authority:?}:{code:?}"),
            message,
        }
    })
}

fn string_list(ctx: &Context, list: bindings::PROJ_STRING_LIST) -> Vec<String> {
    if list.is_null() {
        return Vec::new();
    }
    let mut values = Vec::new();
    let mut index = 0;
    loop {
        // SAFETY: PROJ returns a null-terminated list of valid C strings.
        let item = unsafe { *list.add(index) };
        if item.is_null() {
            break;
        }
        if let Some(value) = cstr_opt(item) {
            values.push(value);
        }
        index += 1;
    }
    // SAFETY: list came from PROJ and is released by its matching destructor.
    unsafe { bindings::proj_string_list_destroy(list) };
    let _ = ctx;
    values
}

pub(crate) fn database_authorities(ctx: &Context) -> Vec<String> {
    // SAFETY: returns a PROJ-owned null-terminated list.
    let list = unsafe { bindings::proj_get_authorities_from_database(ctx.as_ptr()) };
    string_list(ctx, list)
}

pub(crate) fn database_codes(ctx: &Context, authority: &str) -> Result<Vec<String>> {
    let authority = CString::new(authority.as_bytes()).map_err(ProxiError::from)?;
    // SAFETY: authority is valid and PROJ returns a PROJ-owned list.
    let list = unsafe {
        bindings::proj_get_codes_from_database(
            ctx.as_ptr(),
            authority.as_ptr(),
            bindings::PJ_TYPE_CRS,
            0,
        )
    };
    Ok(string_list(ctx, list))
}

pub(crate) fn database_codes_of_type(
    ctx: &Context,
    authority: &str,
    object_type: crate::database::DatabaseType,
) -> Result<Vec<String>> {
    let authority = CString::new(authority.as_bytes()).map_err(ProxiError::from)?;
    let type_ = match object_type {
        crate::database::DatabaseType::Crs => bindings::PJ_TYPE_CRS,
        crate::database::DatabaseType::DatumEnsemble => bindings::PJ_TYPE_DATUM_ENSEMBLE,
        crate::database::DatabaseType::ConcatenatedOperation => {
            bindings::PJ_TYPE_CONCATENATED_OPERATION
        }
        crate::database::DatabaseType::OtherCoordinateOperation => {
            bindings::PJ_TYPE_OTHER_COORDINATE_OPERATION
        }
    };
    let list = unsafe {
        bindings::proj_get_codes_from_database(ctx.as_ptr(), authority.as_ptr(), type_, 0)
    };
    Ok(string_list(ctx, list))
}

pub(crate) fn database_ellipsoids() -> Vec<crate::database::Ellipsoid> {
    let list = unsafe { bindings::proj_list_ellps() };
    let mut values = Vec::new();
    let mut index = 0;
    while index < 10000 {
        let item = unsafe { &*list.add(index) };
        if item.id.is_null() {
            break;
        }
        values.push(crate::database::Ellipsoid {
            id: cstr_opt(item.id).unwrap_or_default(),
            major: cstr_opt(item.major).unwrap_or_default(),
            ellipsoid: cstr_opt(item.ell).unwrap_or_default(),
            name: cstr_opt(item.name).unwrap_or_default(),
        });
        index += 1;
    }
    values
}

pub(crate) fn database_units(angular: bool) -> Vec<crate::database::Unit> {
    let list = unsafe {
        if angular {
            bindings::proj_list_angular_units()
        } else {
            bindings::proj_list_units()
        }
    };
    let mut values = Vec::new();
    let mut index = 0;
    while index < 10000 {
        let item = unsafe { &*list.add(index) };
        if item.id.is_null() {
            break;
        }
        values.push(crate::database::Unit {
            id: cstr_opt(item.id).unwrap_or_default(),
            to_meter: cstr_opt(item.to_meter).unwrap_or_default(),
            name: cstr_opt(item.name).unwrap_or_default(),
            factor: item.factor,
        });
        index += 1;
    }
    values
}

pub(crate) fn database_prime_meridians() -> Vec<crate::database::PrimeMeridian> {
    let list = unsafe { bindings::proj_list_prime_meridians() };
    let mut values = Vec::new();
    let mut index = 0;
    while index < 10000 {
        let item = unsafe { &*list.add(index) };
        if item.id.is_null() {
            break;
        }
        values.push(crate::database::PrimeMeridian {
            id: cstr_opt(item.id).unwrap_or_default(),
            definition: cstr_opt(item.defn).unwrap_or_default(),
        });
        index += 1;
    }
    values
}

pub(crate) fn database_operations() -> Vec<crate::database::Operation> {
    let list = unsafe { bindings::proj_list_operations() };
    let mut values = Vec::new();
    let mut index = 0;
    while index < 10000 {
        let item = unsafe { &*list.add(index) };
        if item.id.is_null() {
            break;
        }
        let description = if item.descr.is_null() {
            None
        } else {
            let first = unsafe { *item.descr };
            cstr_opt(first)
        };
        values.push(crate::database::Operation {
            id: cstr_opt(item.id).unwrap_or_default(),
            description,
        });
        index += 1;
    }
    values
}

// M4.5: database structured search / lookups

/// Execute a filtered CRS search against the database.
///
/// `types` filters the `PJ_TYPE` kinds (empty = all CRS types). `allow_deprecated`
/// includes deprecated codes; `area` optionally constrains to a bounding box;
/// `celestial_body` restricts to an astronomical body (or all when `None`).
/// Returns a `Vec` of owned [`CrsInfoRecord`]s.
pub(crate) fn database_crs_info_list(
    ctx: &Context,
    authority: Option<&str>,
    types: &[bindings::PJ_TYPE],
    allow_deprecated: bool,
    area: Option<[f64; 4]>,
    celestial_body: Option<&str>,
) -> Result<Vec<crate::database::CrsInfoRecord>> {
    let authority = authority
        .map(|value| CString::new(value.as_bytes()).map_err(ProxiError::from))
        .transpose()?;
    let celestial_body = celestial_body
        .map(|value| CString::new(value.as_bytes()).map_err(ProxiError::from))
        .transpose()?;
    // SAFETY: `proj_get_crs_list_parameters_create` returns an owned params struct or null.
    let params = unsafe { bindings::proj_get_crs_list_parameters_create() };
    let params = NonNull::new(params).ok_or_else(|| ProxiError::MissingData {
        message: "failed to create CRS list parameters".to_string(),
    })?;
    let types_copy: Vec<bindings::PJ_TYPE> = types.to_vec();
    // SAFETY: `params` is owned for this scope; populate its plain C fields
    // directly (the `proj_get_crs_list_parameters_set_*` setters are not
    // generated by bindgen, but the struct fields are public).
    unsafe {
        let p = params.as_ptr();
        (*p).types = types_copy.as_ptr();
        (*p).typesCount = types_copy.len();
        (*p).crs_area_of_use_contains_bbox = 0;
        (*p).bbox_valid = area.is_some() as i32;
        if let Some([west, south, east, north]) = area {
            (*p).west_lon_degree = west;
            (*p).south_lat_degree = south;
            (*p).east_lon_degree = east;
            (*p).north_lat_degree = north;
        }
        (*p).allow_deprecated = allow_deprecated as i32;
        (*p).celestial_body_name = celestial_body
            .as_ref()
            .map_or(std::ptr::null(), |s| s.as_ptr());
    }
    // `types_copy` and `celestial_body` must remain alive for the call.
    let _ = &types_copy;
    let _ = &celestial_body;
    let authority_ptr = authority
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());
    let mut count: i32 = 0;
    // SAFETY: authority/params live for the call; PROJ returns an owned list.
    let list = unsafe {
        bindings::proj_get_crs_info_list_from_database(
            ctx.as_ptr(),
            authority_ptr,
            params.as_ptr(),
            &mut count,
        )
    };
    // SAFETY: params no longer needed after the query.
    unsafe { bindings::proj_get_crs_list_parameters_destroy(params.as_ptr()) };
    let list = NonNull::new(list).ok_or_else(|| ProxiError::MissingData {
        message: "proj_get_crs_info_list_from_database returned null".to_string(),
    })?;
    let mut records = Vec::with_capacity(count.max(0) as usize);
    for index in 0..count {
        // SAFETY: `list` is a PROJ-owned array of pointers to `PROJ_CRS_INFO`.
        let item = unsafe { &**list.as_ptr().add(index as usize) };
        records.push(crate::database::CrsInfoRecord {
            authority: cstr_opt(item.auth_name),
            code: cstr_opt(item.code),
            name: cstr_opt(item.name),
            r#type: crate::crs::CrsType::from_raw(item.type_),
            area_name: cstr_opt(item.area_name),
            west_lon_degree: item.west_lon_degree,
            south_lat_degree: item.south_lat_degree,
            east_lon_degree: item.east_lon_degree,
            north_lat_degree: item.north_lat_degree,
            bbox_valid: item.bbox_valid != 0,
            deprecated: item.deprecated != 0,
            celestial_body_name: cstr_opt(item.celestial_body_name),
        });
    }
    // SAFETY: `proj_crs_info_list_destroy` releases the whole array.
    unsafe { bindings::proj_crs_info_list_destroy(list.as_ptr()) };
    Ok(records)
}

/// Enumerate units (and other simple records) from the database.
pub(crate) fn database_units_from_database(
    ctx: &Context,
    authority: Option<&str>,
    category: &str,
    allow_deprecated: bool,
) -> Result<Vec<crate::database::UnitRecord>> {
    let authority = authority
        .map(|value| CString::new(value.as_bytes()).map_err(ProxiError::from))
        .transpose()?;
    let category = CString::new(category.as_bytes()).map_err(ProxiError::from)?;
    let mut count: i32 = 0;
    let authority_ptr = authority
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());
    // SAFETY: strings live for the call; PROJ returns an owned list.
    let list = unsafe {
        bindings::proj_get_units_from_database(
            ctx.as_ptr(),
            authority_ptr,
            category.as_ptr(),
            allow_deprecated as i32,
            &mut count,
        )
    };
    let list = NonNull::new(list).ok_or_else(|| ProxiError::MissingData {
        message: "proj_get_units_from_database returned null".to_string(),
    })?;
    let mut records = Vec::with_capacity(count.max(0) as usize);
    for index in 0..count {
        // SAFETY: `list` is a PROJ-owned array of `PROJ_UNIT_INFO`.
        let item = unsafe { &**list.as_ptr().add(index as usize) };
        records.push(crate::database::UnitRecord {
            authority: cstr_opt(item.auth_name),
            code: cstr_opt(item.code),
            name: cstr_opt(item.name),
            category: cstr_opt(item.category),
            conversion_factor: item.conv_factor,
            proj_short_name: cstr_opt(item.proj_short_name),
            deprecated: item.deprecated != 0,
        });
    }
    // SAFETY: `proj_unit_list_destroy` releases the array.
    unsafe { bindings::proj_unit_list_destroy(list.as_ptr()) };
    Ok(records)
}

/// List geoid-model names for a geographic CRS (authority + code), e.g. the
/// vertical models referenced by a compound operation.
pub(crate) fn database_geoid_models(
    ctx: &Context,
    auth_name: &str,
    code: &str,
) -> Result<Vec<String>> {
    let auth_name = CString::new(auth_name.as_bytes()).map_err(ProxiError::from)?;
    let code = CString::new(code.as_bytes()).map_err(ProxiError::from)?;
    // SAFETY: strings live for the call; PROJ returns a string list (or null).
    let list = unsafe {
        bindings::proj_get_geoid_models_from_database(
            ctx.as_ptr(),
            auth_name.as_ptr(),
            code.as_ptr(),
            std::ptr::null(),
        )
    };
    Ok(string_list(ctx, list))
}

/// Resolve the full crate-level grid metadata for a grid name via the database.
pub(crate) fn database_grid_info(
    ctx: &Context,
    grid_name: &str,
) -> Result<crate::transform::GridInfo> {
    let grid_name = CString::new(grid_name.as_bytes()).map_err(ProxiError::from)?;
    let mut full_name = std::ptr::null();
    let mut package_name = std::ptr::null();
    let mut url = std::ptr::null();
    let mut direct_download = 0;
    let mut open_license = 0;
    let mut available = 0;
    // SAFETY: output pointers are valid for the call.
    let status = unsafe {
        bindings::proj_grid_get_info_from_database(
            ctx.as_ptr(),
            grid_name.as_ptr(),
            &mut full_name,
            &mut package_name,
            &mut url,
            &mut direct_download,
            &mut open_license,
            &mut available,
        )
    };
    if status == 0 {
        return Err(ProxiError::MissingData {
            message: format!("grid {grid_name:?} not found in database"),
        });
    }
    let available = available != 0;
    let direct_download = direct_download != 0;
    let readiness = grid_readiness(ctx, available, direct_download);
    Ok(crate::transform::GridInfo {
        short_name: Some(grid_name.to_string_lossy().into_owned()),
        full_name: cstr_opt(full_name),
        package_name: cstr_opt(package_name),
        url: cstr_opt(url),
        direct_download,
        open_license: open_license != 0,
        readiness,
    })
}

/// Create a coordinate operation (transformer) between two CRSs.
pub(crate) fn create_crs_to_crs(
    ctx: &Context,
    source: &str,
    target: &str,
    area: Option<&AreaBox>,
    options: &[String],
) -> Result<ProjObj> {
    let source_obj = create(ctx, source)?;
    let target_obj = create(ctx, target)?;
    let c_options: Vec<CString> = options
        .iter()
        .map(|option| CString::new(option.as_bytes()).map_err(ProxiError::from))
        .collect::<Result<_>>()?;
    let mut option_ptrs: Vec<*const std::ffi::c_char> =
        c_options.iter().map(|option| option.as_ptr()).collect();
    option_ptrs.push(std::ptr::null());

    // SAFETY: source/target are live CRS objects, the area and option pointers
    // remain valid for the duration of the call, and PROJ returns an owned PJ.
    let pj = unsafe {
        bindings::proj_create_crs_to_crs_from_pj(
            ctx.as_ptr(),
            source_obj.as_ptr(),
            target_obj.as_ptr(),
            area.map_or(std::ptr::null_mut(), |a| a.as_ptr()),
            option_ptrs.as_ptr(),
        )
    };
    drop(source_obj);
    drop(target_obj);
    ProjObj::new(pj, NonNull::new(ctx.as_ptr()).expect("live context")).ok_or_else(|| {
        let (_, msg) = ctx.errno_message();
        ProxiError::InvalidTransformer {
            source_crs: source.to_string(),
            target_crs: target.to_string(),
            message: msg,
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_operation_candidates(
    ctx: &Context,
    source: &str,
    target: &str,
    authority: Option<&str>,
    desired_accuracy: Option<f64>,
    area: Option<[f64; 4]>,
    discard_superseded: bool,
    allow_ballpark: Option<bool>,
    crs_extent_use: Option<crate::transform::CrsExtentUse>,
    spatial_criterion: Option<crate::transform::SpatialCriterion>,
    grid_availability_use: Option<crate::transform::GridAvailabilityUse>,
    use_proj_alternative_grid_names: Option<bool>,
) -> Result<Vec<ProjObj>> {
    let source_obj = create(ctx, source)?;
    let target_obj = create(ctx, target)?;
    let authority = authority
        .map(|value| CString::new(value.as_bytes()).map_err(ProxiError::from))
        .transpose()?;
    let authority_ptr = authority
        .as_ref()
        .map_or(std::ptr::null(), |value| value.as_ptr());
    // SAFETY: context and optional authority are valid for this call.
    let factory =
        unsafe { bindings::proj_create_operation_factory_context(ctx.as_ptr(), authority_ptr) };
    let factory = NonNull::new(factory).ok_or_else(|| ProxiError::InvalidTransformer {
        source_crs: source.to_string(),
        target_crs: target.to_string(),
        message: "failed to create operation factory context".to_string(),
    })?;
    if let Some(accuracy) = desired_accuracy {
        // SAFETY: factory is live and owned until the end of this scope.
        unsafe {
            bindings::proj_operation_factory_context_set_desired_accuracy(
                ctx.as_ptr(),
                factory.as_ptr(),
                accuracy,
            );
        }
    }
    if let Some(area) = area {
        // SAFETY: area and factory are live for the call.
        unsafe {
            bindings::proj_operation_factory_context_set_area_of_interest(
                ctx.as_ptr(),
                factory.as_ptr(),
                area[0],
                area[1],
                area[2],
                area[3],
            );
        }
    }
    // SAFETY: factory is live for the call.
    unsafe {
        bindings::proj_operation_factory_context_set_discard_superseded(
            ctx.as_ptr(),
            factory.as_ptr(),
            discard_superseded as i32,
        );
        if let Some(allow) = allow_ballpark {
            bindings::proj_operation_factory_context_set_allow_ballpark_transformations(
                ctx.as_ptr(),
                factory.as_ptr(),
                allow as i32,
            );
        }
        if let Some(ext) = crs_extent_use {
            let v = match ext {
                crate::transform::CrsExtentUse::None => {
                    bindings::PROJ_CRS_EXTENT_USE_PJ_CRS_EXTENT_NONE
                }
                crate::transform::CrsExtentUse::Both => {
                    bindings::PROJ_CRS_EXTENT_USE_PJ_CRS_EXTENT_BOTH
                }
                crate::transform::CrsExtentUse::Intersection => {
                    bindings::PROJ_CRS_EXTENT_USE_PJ_CRS_EXTENT_INTERSECTION
                }
                crate::transform::CrsExtentUse::Smallest => {
                    bindings::PROJ_CRS_EXTENT_USE_PJ_CRS_EXTENT_SMALLEST
                }
            };
            bindings::proj_operation_factory_context_set_crs_extent_use(
                ctx.as_ptr(),
                factory.as_ptr(),
                v,
            );
        }
        if let Some(c) = spatial_criterion {
            let v = match c {
                crate::transform::SpatialCriterion::StrictContainment => {
                    bindings::PROJ_SPATIAL_CRITERION_PROJ_SPATIAL_CRITERION_STRICT_CONTAINMENT
                }
                crate::transform::SpatialCriterion::PartialIntersection => {
                    bindings::PROJ_SPATIAL_CRITERION_PROJ_SPATIAL_CRITERION_PARTIAL_INTERSECTION
                }
            };
            bindings::proj_operation_factory_context_set_spatial_criterion(
                ctx.as_ptr(),
                factory.as_ptr(),
                v,
            );
        }
        if let Some(ga) = grid_availability_use {
            let v = match ga {
                crate::transform::GridAvailabilityUse::UsedForSorting => bindings::PROJ_GRID_AVAILABILITY_USE_PROJ_GRID_AVAILABILITY_USED_FOR_SORTING,
                crate::transform::GridAvailabilityUse::DiscardOperationIfMissingGrid => bindings::PROJ_GRID_AVAILABILITY_USE_PROJ_GRID_AVAILABILITY_DISCARD_OPERATION_IF_MISSING_GRID,
                crate::transform::GridAvailabilityUse::Ignored => bindings::PROJ_GRID_AVAILABILITY_USE_PROJ_GRID_AVAILABILITY_IGNORED,
                crate::transform::GridAvailabilityUse::KnownAvailable => bindings::PROJ_GRID_AVAILABILITY_USE_PROJ_GRID_AVAILABILITY_KNOWN_AVAILABLE,
            };
            bindings::proj_operation_factory_context_set_grid_availability_use(
                ctx.as_ptr(),
                factory.as_ptr(),
                v,
            );
        }
        if let Some(use_alt) = use_proj_alternative_grid_names {
            bindings::proj_operation_factory_context_set_use_proj_alternative_grid_names(
                ctx.as_ptr(),
                factory.as_ptr(),
                use_alt as i32,
            );
        }
    }
    // SAFETY: source/target and factory are live for the call.
    let list = unsafe {
        bindings::proj_create_operations(
            ctx.as_ptr(),
            source_obj.as_ptr(),
            target_obj.as_ptr(),
            factory.as_ptr(),
        )
    };
    // SAFETY: factory context is no longer needed after the list is created.
    unsafe { bindings::proj_operation_factory_context_destroy(factory.as_ptr()) };
    let list = NonNull::new(list).ok_or_else(|| ProxiError::InvalidTransformer {
        source_crs: source.to_string(),
        target_crs: target.to_string(),
        message: "PROJ returned no operation list".to_string(),
    })?;
    let count = unsafe { bindings::proj_list_get_count(list.as_ptr()) };
    let mut operations = Vec::with_capacity(count.max(0) as usize);
    for index in 0..count {
        // SAFETY: index is within the list count; PROJ returns an owned PJ.
        let operation = unsafe { bindings::proj_list_get(ctx.as_ptr(), list.as_ptr(), index) };
        let Some(operation) =
            ProjObj::new(operation, NonNull::new(ctx.as_ptr()).expect("live context"))
        else {
            unsafe { bindings::proj_list_destroy(list.as_ptr()) };
            return Err(ProxiError::InvalidTransformer {
                source_crs: source.to_string(),
                target_crs: target.to_string(),
                message: format!("PROJ returned null operation at index {index}"),
            });
        };
        operations.push(operation);
    }
    // SAFETY: the list is no longer needed after its PJ objects are copied out.
    unsafe { bindings::proj_list_destroy(list.as_ptr()) };
    Ok(operations)
}

/// Create a coordinate operation from an explicit PROJ pipeline string.
pub(crate) fn create_pipeline(ctx: &Context, pipeline: &str) -> Result<ProjObj> {
    let c_pipeline = CString::new(pipeline.as_bytes()).map_err(ProxiError::from)?;
    // SAFETY: The string is valid for the duration of the call and PROJ
    // returns an owned operation object or null.
    let pj = unsafe { bindings::proj_create(ctx.as_ptr(), c_pipeline.as_ptr()) };
    ProjObj::new(pj, NonNull::new(ctx.as_ptr()).expect("live context")).ok_or_else(|| {
        let (_, msg) = ctx.errno_message();
        ProxiError::InvalidTransformer {
            source_crs: "<pipeline>".to_string(),
            target_crs: "<pipeline>".to_string(),
            message: msg,
        }
    })
}

/// Read operation metadata. The returned pointers are owned by PROJ and are
/// valid for the lifetime of the operation; callers must copy them promptly.
pub(crate) fn pj_info(obj: &ProjObj) -> bindings::PJ_PROJ_INFO {
    // SAFETY: `obj` is a live PROJ operation.
    unsafe { bindings::proj_pj_info(obj.as_ptr()) }
}

pub(crate) fn cstr_opt(ptr: *const std::ffi::c_char) -> Option<String> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: PROJ returns a valid NUL-terminated string or null.
        Some(
            unsafe { std::ffi::CStr::from_ptr(ptr) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}

#[allow(clippy::type_complexity)]
pub(crate) fn area_of_use(
    obj: &ProjObj,
    ctx: &Context,
) -> Result<Option<(f64, f64, f64, f64, Option<String>)>> {
    let mut west = 0.0;
    let mut south = 0.0;
    let mut east = 0.0;
    let mut north = 0.0;
    let mut name = std::ptr::null();
    // SAFETY: output pointers are valid for the duration of the call.
    let status = unsafe {
        bindings::proj_get_area_of_use(
            ctx.as_ptr(),
            obj.as_ptr(),
            &mut west,
            &mut south,
            &mut east,
            &mut north,
            &mut name,
        )
    };
    if status == 0 {
        return Ok(None);
    }
    Ok(Some((west, south, east, north, cstr_opt(name))))
}

pub(crate) fn operation_grids(
    obj: &ProjObj,
    ctx: &Context,
) -> Result<Vec<crate::transform::GridInfo>> {
    let count =
        unsafe { bindings::proj_coordoperation_get_grid_used_count(ctx.as_ptr(), obj.as_ptr()) };
    if count < 0 {
        return Err(ProxiError::Transform {
            code: 0,
            message: "PROJ could not inspect operation grids".to_string(),
        });
    }
    let mut grids = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut short_name = std::ptr::null();
        let mut full_name = std::ptr::null();
        let mut package_name = std::ptr::null();
        let mut url = std::ptr::null();
        let mut direct_download = 0;
        let mut open_license = 0;
        let mut available = 0;
        let status = unsafe {
            bindings::proj_coordoperation_get_grid_used(
                ctx.as_ptr(),
                obj.as_ptr(),
                index,
                &mut short_name,
                &mut full_name,
                &mut package_name,
                &mut url,
                &mut direct_download,
                &mut open_license,
                &mut available,
            )
        };
        if status == 0 {
            return Err(ProxiError::Transform {
                code: 0,
                message: format!("PROJ could not inspect operation grid {index}"),
            });
        }
        let available = available != 0;
        let direct_download = direct_download != 0;
        let readiness = grid_readiness(ctx, available, direct_download);
        grids.push(crate::transform::GridInfo {
            short_name: cstr_opt(short_name),
            full_name: cstr_opt(full_name),
            package_name: cstr_opt(package_name),
            url: cstr_opt(url),
            direct_download,
            open_license: open_license != 0,
            readiness,
        });
    }
    Ok(grids)
}

fn grid_readiness(
    ctx: &Context,
    available: bool,
    direct_download: bool,
) -> crate::transform::GridReadiness {
    if available {
        return crate::transform::GridReadiness::Ready;
    }
    if !direct_download {
        return crate::transform::GridReadiness::Unavailable;
    }
    #[cfg(feature = "network")]
    if ctx.network_enabled() {
        return crate::transform::GridReadiness::Downloadable;
    }
    crate::transform::GridReadiness::NetworkDisabled
}

pub(crate) fn operation_crs_wkts(obj: &ProjObj, ctx: &Context) -> (Option<String>, Option<String>) {
    let source = unsafe { bindings::proj_get_source_crs(ctx.as_ptr(), obj.as_ptr()) };
    let source = ProjObj::new(source, obj.context)
        .and_then(|crs| as_wkt(&crs, crate::options::WktVersion::Wkt2_2019, None).ok());
    let target = unsafe { bindings::proj_get_target_crs(ctx.as_ptr(), obj.as_ptr()) };
    let target = ProjObj::new(target, obj.context)
        .and_then(|crs| as_wkt(&crs, crate::options::WktVersion::Wkt2_2019, None).ok());
    (source, target)
}

/// Whether a coordinate operation is instantiable (computable). Returns `None`
/// for operation types that don't support the query.
pub(crate) fn operation_instantiable(obj: &ProjObj, ctx: &Context) -> Option<bool> {
    // SAFETY: `proj_coordoperation_is_instantiable` takes a live PJ.
    unsafe {
        let b = bindings::proj_coordoperation_is_instantiable(ctx.as_ptr(), obj.as_ptr());
        match b {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

/// Enumerate the parameters of a coordinate operation's method.
///
/// Capability-checked: only valid for `CONVERSION` or `TRANSFORMATION`
/// operations; returns empty for other types (PROJ has no parameters there).
pub(crate) fn operation_parameters(
    obj: &ProjObj,
    ctx: &Context,
) -> Vec<crate::transform::OperationParameter> {
    let object_type = unsafe { bindings::proj_get_type(obj.as_ptr()) };
    if object_type != bindings::PJ_TYPE_CONVERSION
        && object_type != bindings::PJ_TYPE_TRANSFORMATION
    {
        return Vec::new();
    }
    let count =
        unsafe { bindings::proj_coordoperation_get_param_count(ctx.as_ptr(), obj.as_ptr()) };
    if count < 0 {
        return Vec::new();
    }
    let mut params = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut name: *const std::ffi::c_char = std::ptr::null();
        let mut auth: *const std::ffi::c_char = std::ptr::null();
        let mut code: *const std::ffi::c_char = std::ptr::null();
        let mut value = 0.0;
        let mut value_string: *const std::ffi::c_char = std::ptr::null();
        let mut unit_factor = 0.0;
        let mut unit_name: *const std::ffi::c_char = std::ptr::null();
        let mut unit_auth: *const std::ffi::c_char = std::ptr::null();
        let mut unit_code: *const std::ffi::c_char = std::ptr::null();
        let mut unit_category: *const std::ffi::c_char = std::ptr::null();
        // SAFETY: all output pointers are valid for the call.
        let status = unsafe {
            bindings::proj_coordoperation_get_param(
                ctx.as_ptr(),
                obj.as_ptr(),
                index,
                &mut name,
                &mut auth,
                &mut code,
                &mut value,
                &mut value_string,
                &mut unit_factor,
                &mut unit_name,
                &mut unit_auth,
                &mut unit_code,
                &mut unit_category,
            )
        };
        if status == 0 {
            // Skip parameters PROJ didn't return; continue enumerating.
            continue;
        }
        params.push(crate::transform::OperationParameter {
            name: cstr_opt(name),
            authority: cstr_opt(auth),
            code: cstr_opt(code),
            value: Some(value),
            value_string: cstr_opt(value_string),
            unit_conversion_factor: Some(unit_factor),
            unit_name: cstr_opt(unit_name),
            unit_authority: cstr_opt(unit_auth),
            unit_code: cstr_opt(unit_code),
            unit_category: cstr_opt(unit_category),
        });
    }
    params
}

pub(crate) fn operation_method(
    obj: &ProjObj,
    ctx: &Context,
) -> (Option<String>, Option<String>, Option<String>, Option<bool>) {
    let object_type = unsafe { bindings::proj_get_type(obj.as_ptr()) };
    if object_type != bindings::PJ_TYPE_CONVERSION
        && object_type != bindings::PJ_TYPE_TRANSFORMATION
    {
        return (None, None, None, None);
    }
    let mut name = std::ptr::null();
    let mut authority = std::ptr::null();
    let mut code = std::ptr::null();
    let status = unsafe {
        bindings::proj_coordoperation_get_method_info(
            ctx.as_ptr(),
            obj.as_ptr(),
            &mut name,
            &mut authority,
            &mut code,
        )
    };
    let method = if status != 0 {
        (cstr_opt(name), cstr_opt(authority), cstr_opt(code))
    } else {
        (None, None, None)
    };
    let ballpark = unsafe {
        bindings::proj_coordoperation_has_ballpark_transformation(ctx.as_ptr(), obj.as_ptr())
    };
    (method.0, method.1, method.2, Some(ballpark != 0))
}

pub(crate) fn crs_name(obj: &ProjObj) -> Option<String> {
    // SAFETY: `obj` is a live CRS object.
    cstr_opt(unsafe { bindings::proj_get_name(obj.as_ptr()) })
}

pub(crate) fn crs_identifiers(obj: &ProjObj) -> Vec<(String, String)> {
    let mut identifiers = Vec::new();
    for index in 0..32 {
        // SAFETY: PROJ accepts non-negative identifier indexes and returns
        // null when no identifier exists at that index.
        let authority = unsafe { bindings::proj_get_id_auth_name(obj.as_ptr(), index) };
        let code = unsafe { bindings::proj_get_id_code(obj.as_ptr(), index) };
        let (Some(authority), Some(code)) = (cstr_opt(authority), cstr_opt(code)) else {
            break;
        };
        identifiers.push((authority, code));
    }
    identifiers
}

pub(crate) fn crs_scope(obj: &ProjObj) -> Option<String> {
    // SAFETY: `obj` is a live CRS object.
    cstr_opt(unsafe { bindings::proj_get_scope(obj.as_ptr()) })
}

pub(crate) fn crs_remarks(obj: &ProjObj) -> Option<String> {
    // SAFETY: `obj` is a live CRS object.
    cstr_opt(unsafe { bindings::proj_get_remarks(obj.as_ptr()) })
}

pub(crate) fn crs_equivalent(
    ctx: &Context,
    obj: &ProjObj,
    other: &ProjObj,
    criterion: bindings::PJ_COMPARISON_CRITERION,
) -> bool {
    // SAFETY: both objects belong to live PROJ contexts and the comparison
    // function only reads them.
    unsafe {
        bindings::proj_is_equivalent_to_with_ctx(
            ctx.as_ptr(),
            obj.as_ptr(),
            other.as_ptr(),
            criterion,
        ) != 0
    }
}

pub(crate) fn crs_ellipsoid(obj: &ProjObj, ctx: &Context) -> Option<(f64, f64, f64, bool)> {
    let raw = unsafe { bindings::proj_get_ellipsoid(ctx.as_ptr(), obj.as_ptr()) };
    let ellipsoid = ProjObj::new(raw, obj.context)?;
    let mut major = 0.0;
    let mut minor = 0.0;
    let mut computed = 0;
    let mut inverse_flattening = 0.0;
    let status = unsafe {
        bindings::proj_ellipsoid_get_parameters(
            ctx.as_ptr(),
            ellipsoid.as_ptr(),
            &mut major,
            &mut minor,
            &mut computed,
            &mut inverse_flattening,
        )
    };
    (status != 0).then_some((major, minor, inverse_flattening, computed != 0))
}

pub(crate) fn crs_prime_meridian(
    obj: &ProjObj,
    ctx: &Context,
) -> Option<(f64, f64, Option<String>)> {
    let raw = unsafe { bindings::proj_get_prime_meridian(ctx.as_ptr(), obj.as_ptr()) };
    let meridian = ProjObj::new(raw, obj.context)?;
    let mut longitude = 0.0;
    let mut factor = 0.0;
    let mut unit_name = std::ptr::null();
    let status = unsafe {
        bindings::proj_prime_meridian_get_parameters(
            ctx.as_ptr(),
            meridian.as_ptr(),
            &mut longitude,
            &mut factor,
            &mut unit_name,
        )
    };
    (status != 0).then_some((longitude, factor, cstr_opt(unit_name)))
}

pub(crate) fn crs_datum_ensemble(
    obj: &ProjObj,
    ctx: &Context,
) -> Option<(Option<String>, f64, Vec<Option<String>>)> {
    let raw = unsafe { bindings::proj_crs_get_datum_ensemble(ctx.as_ptr(), obj.as_ptr()) };
    let ensemble = ProjObj::new(raw, NonNull::new(ctx.as_ptr()).expect("live context"))?;
    let count =
        unsafe { bindings::proj_datum_ensemble_get_member_count(ctx.as_ptr(), ensemble.as_ptr()) };
    if count < 0 {
        return None;
    }
    let accuracy =
        unsafe { bindings::proj_datum_ensemble_get_accuracy(ctx.as_ptr(), ensemble.as_ptr()) };
    let mut members = Vec::with_capacity(count as usize);
    for index in 0..count {
        let member = unsafe {
            bindings::proj_datum_ensemble_get_member(ctx.as_ptr(), ensemble.as_ptr(), index)
        };
        let member = ProjObj::new(member, NonNull::new(ctx.as_ptr()).expect("live context"));
        members.push(
            member
                .as_ref()
                .and_then(|value| cstr_opt(pj_info(value).id)),
        );
    }
    Some((crs_name(&ensemble), accuracy, members))
}

#[allow(clippy::type_complexity)]
pub(crate) fn crs_coordinate_system(
    obj: &ProjObj,
    ctx: &Context,
) -> Option<(
    bindings::PJ_COORDINATE_SYSTEM_TYPE,
    Vec<(
        Option<String>,
        Option<String>,
        Option<String>,
        f64,
        Option<String>,
        Option<String>,
        Option<String>,
    )>,
)> {
    // SAFETY: PROJ returns an owned coordinate-system object or null.
    let raw = unsafe { bindings::proj_crs_get_coordinate_system(ctx.as_ptr(), obj.as_ptr()) };
    let cs = ProjObj::new(raw, obj.context)?;
    // SAFETY: `cs` is a live coordinate-system object.
    let kind = unsafe { bindings::proj_cs_get_type(ctx.as_ptr(), cs.as_ptr()) };
    let count = unsafe { bindings::proj_cs_get_axis_count(ctx.as_ptr(), cs.as_ptr()) };
    if count < 0 {
        return None;
    }
    let mut axes = Vec::with_capacity(count as usize);
    for index in 0..count {
        let mut name = std::ptr::null();
        let mut abbreviation = std::ptr::null();
        let mut direction = std::ptr::null();
        let mut conversion_factor = 0.0;
        let mut unit_name = std::ptr::null();
        let mut unit_authority = std::ptr::null();
        let mut unit_code = std::ptr::null();
        // SAFETY: all output pointers are valid for the call.
        let status = unsafe {
            bindings::proj_cs_get_axis_info(
                ctx.as_ptr(),
                cs.as_ptr(),
                index,
                &mut name,
                &mut abbreviation,
                &mut direction,
                &mut conversion_factor,
                &mut unit_name,
                &mut unit_authority,
                &mut unit_code,
            )
        };
        if status == 0 {
            return None;
        }
        axes.push((
            cstr_opt(name),
            cstr_opt(abbreviation),
            cstr_opt(direction),
            conversion_factor,
            cstr_opt(unit_name),
            cstr_opt(unit_authority),
            cstr_opt(unit_code),
        ));
    }
    Some((kind, axes))
}

/// Normalize a transformer for visualization (sets axis order to x/y and
/// wraps in a "transformation" for display). Consumes the input object and
/// returns a new one.
pub(crate) fn normalize_for_visualization(ctx: &Context, obj: ProjObj) -> Result<ProjObj> {
    let ptr = obj.as_ptr();
    // SAFETY: `ptr` is a valid `PJ*`. PROJ returns a *new* object and does not
    // consume the input; the caller must still destroy the original.
    let normalized = unsafe { bindings::proj_normalize_for_visualization(ctx.as_ptr(), ptr) };
    let Some(new_obj) = ProjObj::new(
        normalized,
        NonNull::new(ctx.as_ptr()).expect("live context"),
    ) else {
        return Err(ProxiError::InvalidTransformer {
            source_crs: String::new(),
            target_crs: String::new(),
            message: "normalize_for_visualization failed (operation may not support always_xy)"
                .to_string(),
        });
    };
    // The original must be destroyed (PROJ did not take ownership).
    drop(obj);
    Ok(new_obj)
}

/// Area-of-interest (bbox) wrapper with RAII.
pub(crate) struct AreaBox {
    raw: NonNull<bindings::PJ_AREA>,
}

impl AreaBox {
    pub(crate) fn new(west: f64, south: f64, east: f64, north: f64) -> Result<Self> {
        // SAFETY: `proj_area_create` returns a new area or null.
        let raw = unsafe { bindings::proj_area_create() };
        let raw = NonNull::new(raw).ok_or_else(|| ProxiError::MissingData {
            message: "proj_area_create failed".to_string(),
        })?;
        // SAFETY: `raw` is a valid `PJ_AREA*`.
        unsafe {
            bindings::proj_area_set_bbox(raw.as_ptr(), west, south, east, north);
        }
        Ok(Self { raw })
    }

    pub(crate) fn as_ptr(&self) -> *mut bindings::PJ_AREA {
        self.raw.as_ptr()
    }
}

impl Drop for AreaBox {
    fn drop(&mut self) {
        // SAFETY: `raw` is an owned `PJ_AREA*`.
        unsafe { bindings::proj_area_destroy(self.raw.as_ptr()) };
    }
}

/// The deg/rad conversion constants (pyproj parity).
pub(crate) const DG2RAD: f64 = std::f64::consts::PI / 180.0;
pub(crate) const RAD2DG: f64 = 180.0 / std::f64::consts::PI;

/// Convert `Direction` to the PROJ direction constant.
pub(crate) fn dir_code(dir: crate::options::Direction) -> bindings::PJ_DIRECTION {
    match dir {
        crate::options::Direction::Forward => bindings::PJ_FWD,
        crate::options::Direction::Inverse => bindings::PJ_INV,
    }
}

/// Transform a single coordinate.
pub(crate) fn trans(obj: &ProjObj, dir: bindings::PJ_DIRECTION, v: [f64; 4]) -> [f64; 4] {
    let coord = bindings::PJ_COORD::xyzt(v[0], v[1], v[2], v[3]);
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    let out = unsafe { bindings::proj_trans(obj.as_ptr(), dir, coord) };
    unsafe { out.v }
}

/// Transform a batch in-place via `proj_trans_generic`.
///
/// `x`, `y`, `z`, `t` are parallel buffers of equal length `n`. Any of
/// `z`/`t` may be omitted (pass `None`), in which case PROJ uses a null/zero
/// stride and the corresponding coordinate is left at its default. Callers
/// must have validated lengths.
pub(crate) fn trans_generic(
    obj: &ProjObj,
    dir: bindings::PJ_DIRECTION,
    x: &mut [f64],
    y: &mut [f64],
    z: Option<&mut [f64]>,
    t: Option<&mut [f64]>,
) -> usize {
    let n = x.len();
    let (z_ptr, sz, nz) = match z {
        Some(z) if z.len() == n => (z.as_mut_ptr(), std::mem::size_of::<f64>(), n),
        _ => (std::ptr::null_mut(), 0, 0),
    };
    let (t_ptr, st, nt) = match t {
        Some(t) if t.len() == n => (t.as_mut_ptr(), std::mem::size_of::<f64>(), n),
        _ => (std::ptr::null_mut(), 0, 0),
    };
    // SAFETY: All pointers are valid for `n` elements; the caller guarantees
    // equal lengths and that the buffers outlive the call.
    unsafe {
        bindings::proj_trans_generic(
            obj.as_ptr(),
            dir,
            x.as_mut_ptr(),
            std::mem::size_of::<f64>(),
            n,
            y.as_mut_ptr(),
            std::mem::size_of::<f64>(),
            n,
            z_ptr,
            sz,
            nz,
            t_ptr,
            st,
            nt,
        )
    }
}

/// Transform an axis-aligned 2D bounds rectangle with optional edge densification.
pub(crate) fn trans_bounds(
    obj: &ProjObj,
    ctx: &Context,
    dir: bindings::PJ_DIRECTION,
    bounds: [f64; 4],
    densify_points: i32,
) -> Result<[f64; 4]> {
    let mut out = [0.0; 4];
    // SAFETY: PROJ receives a live operation/context and valid output pointers.
    let status = unsafe {
        bindings::proj_trans_bounds(
            ctx.as_ptr(),
            obj.as_ptr(),
            dir,
            bounds[0],
            bounds[1],
            bounds[2],
            bounds[3],
            &mut out[0],
            &mut out[1],
            &mut out[2],
            &mut out[3],
            densify_points,
        )
    };
    let code = errno(obj);
    if status == 0 || code != 0 {
        return Err(ProxiError::Transform {
            code,
            message: errno_string(ctx, code),
        });
    }
    Ok(out)
}

/// Whether the operation expects angular (radian) input for the given direction.
pub(crate) fn angular_input(obj: &ProjObj, dir: bindings::PJ_DIRECTION) -> bool {
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    unsafe { bindings::proj_angular_input(obj.as_ptr(), dir) != 0 }
}

/// Whether the operation produces angular (radian) output for the given direction.
pub(crate) fn angular_output(obj: &ProjObj, dir: bindings::PJ_DIRECTION) -> bool {
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    unsafe { bindings::proj_angular_output(obj.as_ptr(), dir) != 0 }
}

/// Whether the operation expects degree input for the given direction.
pub(crate) fn degree_input(obj: &ProjObj, dir: bindings::PJ_DIRECTION) -> bool {
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    unsafe { bindings::proj_degree_input(obj.as_ptr(), dir) != 0 }
}

/// Whether the operation produces degree output for the given direction.
pub(crate) fn degree_output(obj: &ProjObj, dir: bindings::PJ_DIRECTION) -> bool {
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    unsafe { bindings::proj_degree_output(obj.as_ptr(), dir) != 0 }
}

/// Reset the error flag on an operation; returns the previous error number.
pub(crate) fn errno_reset(obj: &ProjObj) -> i32 {
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    unsafe { bindings::proj_errno_reset(obj.as_ptr()) }
}

/// Get the current error number on an operation.
pub(crate) fn errno(obj: &ProjObj) -> i32 {
    // SAFETY: `obj.as_ptr()` is a valid `PJ*`.
    unsafe { bindings::proj_errno(obj.as_ptr()) }
}

/// Get a human-readable message for a PROJ error code on a context.
pub(crate) fn errno_string(ctx: &Context, code: i32) -> String {
    // SAFETY: PROJ returns a static string or null.
    let ptr = unsafe { bindings::proj_context_errno_string(ctx.as_ptr(), code) };
    if ptr.is_null() {
        format!("PROJ error {code}")
    } else {
        // SAFETY: `ptr` is a valid NUL-terminated static string.
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Serialize a PROJ object as a PROJ string (`+proj=...` or `+init=...`).
///
/// `version` selects the WKT2-era (`+proj=...`) or legacy proj.4 form.
/// Returns the PROJ-owned static string (no free required).
pub(crate) fn as_proj_string(
    obj: &ProjObj,
    ctx: &Context,
    version: crate::options::ProjStringVersion,
) -> Result<String> {
    let proj_type = match version {
        crate::options::ProjStringVersion::Proj5 => bindings::PJ_PROJ_STRING_TYPE_PJ_PROJ_5,
        crate::options::ProjStringVersion::Proj4 => bindings::PJ_PROJ_STRING_TYPE_PJ_PROJ_4,
    };
    // SAFETY: `proj_as_proj_string` returns a static string (or null) owned by
    // PROJ; no freeing is required.
    let ptr = unsafe {
        bindings::proj_as_proj_string(ctx.as_ptr(), obj.as_ptr(), proj_type, std::ptr::null())
    };
    if ptr.is_null() {
        return Err(ProxiError::InvalidCrs {
            input: String::new(),
            message: "proj_as_proj_string returned null".to_string(),
        });
    }
    // SAFETY: `ptr` is a valid NUL-terminated static string.
    Ok(unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned())
}

/// Map [`WktOptions`] to PROJ option strings (`KEY=VALUE`).
fn wkt_option_strings(options: &crate::options::WktOptions) -> Vec<String> {
    let mut strings = Vec::new();
    if let Some(multiline) = options.multiline {
        strings.push(if multiline {
            "MULTILINE=YES".to_string()
        } else {
            "MULTILINE=NO".to_string()
        });
    }
    if let Some(width) = options.indentation_width {
        strings.push(format!("INDENTATION_WIDTH={width}"));
    }
    if let Some(allow) = options.allow_ellipsoidal_height_as_vertical_crs {
        strings.push(if allow {
            "ALLOW_ELLIPSOIDAL_HEIGHT_AS_VERTICAL_CRS=YES".to_string()
        } else {
            "ALLOW_ELLIPSOIDAL_HEIGHT_AS_VERTICAL_CRS=NO".to_string()
        });
    }
    if let Some(axis) = options.output_axis_order {
        let value = match axis {
            crate::options::AxisOutputOrder::Traditional => "traditional",
            crate::options::AxisOutputOrder::Authority => "authority",
            crate::options::AxisOutputOrder::Order => "order",
        };
        strings.push(format!("OUTPUT_AXIS={value}"));
    }
    if let Some(output_conversion) = options.output_conversion {
        strings.push(if output_conversion {
            "OUTPUT_CONVERSION=YES".to_string()
        } else {
            "OUTPUT_CONVERSION=NO".to_string()
        });
    }
    if let Some(always_xy) = options.use_always_xy {
        strings.push(if always_xy {
            "USE_ALWAYS_XY=YES".to_string()
        } else {
            "USE_ALWAYS_XY=NO".to_string()
        });
    }
    strings
}

/// Copy a NUL-terminated string returned by PROJ, then free it with
/// `proj_string_destroy` (PROJ's documented `free`).
///
/// For the allocator contract (esp. the MSVC no-free guard) see P0.1:
/// `proj_string_destroy` is literally `free(str)`, so it's only valid if PROJ
/// and Rust share one CRT heap. On non-MSVC the bundled superbuild shares one
/// allocator and the free is safe (verified leak-free in `tests/string_ownership.rs`).
/// On MSVC, even with `/MD`, the statically-linked libproj does not reliably
/// share Rust's CRT heap; freeing corrupts the heap (`0xc0000374`), so we do
/// NOT free (a bounded, documented leak) until PROJ is linked shared on Windows (M8).
fn copy_proj_string(ptr: *const std::ffi::c_char) -> Result<String> {
    if ptr.is_null() {
        return Err(ProxiError::InvalidCrs {
            input: String::new(),
            message: "PROJ returned null".to_string(),
        });
    }
    let s = unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    #[cfg(not(all(windows, target_env = "msvc")))]
    unsafe {
        bindings::proj_string_destroy(ptr.cast_mut());
    }
    Ok(s)
}

/// WKT output for a PROJ object.
pub(crate) fn as_wkt(
    obj: &ProjObj,
    version: crate::options::WktVersion,
    options: Option<&crate::options::WktOptions>,
) -> Result<String> {
    let wkt_type = match version {
        crate::options::WktVersion::Wkt2_2019 => bindings::PJ_WKT2_2019,
        crate::options::WktVersion::Wkt2_2019Simplified => {
            bindings::PJ_WKT_TYPE_PJ_WKT2_2019_SIMPLIFIED
        }
        crate::options::WktVersion::Wkt2_2015 => bindings::PJ_WKT2_2015,
        crate::options::WktVersion::Wkt2_2015Simplified => {
            bindings::PJ_WKT_TYPE_PJ_WKT2_2015_SIMPLIFIED
        }
        crate::options::WktVersion::Wkt1Esri => bindings::PJ_WKT1_ESRI,
        crate::options::WktVersion::Wkt1Gdal => bindings::PJ_WKT1_GDAL,
    };
    let c_strings: Vec<CString> = options
        .map(|options| {
            wkt_option_strings(options)
                .iter()
                .filter_map(|option| CString::new(option.as_bytes()).ok())
                .collect()
        })
        .unwrap_or_default();
    let mut option_ptrs: Vec<*const std::ffi::c_char> =
        c_strings.iter().map(|option| option.as_ptr()).collect();
    option_ptrs.push(std::ptr::null());
    // SAFETY: c_strings and option_ptrs stay alive for the call; PROJ reads
    // them during `proj_as_wkt` and does not retain them.
    let ptr = unsafe {
        bindings::proj_as_wkt(
            obj.context_ptr(),
            obj.as_ptr(),
            wkt_type,
            option_ptrs.as_ptr(),
        )
    };
    // Drop c_strings / option_ptrs only after the call returns.
    drop(c_strings);
    drop(option_ptrs);
    copy_proj_string(ptr)
}

/// PROJJSON output for a PROJ object.
pub(crate) fn as_projjson(obj: &ProjObj) -> Result<String> {
    let ptr =
        unsafe { bindings::proj_as_projjson(obj.context_ptr(), obj.as_ptr(), std::ptr::null()) };
    copy_proj_string(ptr)
}

// M4.3: bespoke CRS / conversion constructors.
//
// A non-CRS object (datum, CS, conversion) is still an owned `PJ*`: drop it
// with proj_destroy. The composite constructors here only read their PJ
// arguments (they take `*const PJ`), so callers must keep those alive for the
// call; the result gets freed when the returned ProjObj drops.

/// Wrap an owned `PJ*` (from any `proj_create_*`) in a context-bound `ProjObj`.
fn owned(ctx: &Context, ptr: *mut bindings::PJ) -> Option<ProjObj> {
    ProjObj::new(ptr, NonNull::new(ctx.as_ptr()).expect("live context"))
}

/// Owned 2D ellipsoidal (geographic) coordinate system.
pub(crate) fn create_ellipsoidal_2d_cs(
    ctx: &Context,
    longitude_latitude: bool,
    unit_name: &str,
    unit_conv_factor: f64,
) -> Option<ProjObj> {
    let type_ = if longitude_latitude {
        bindings::PJ_ELLIPSOIDAL_CS_2D_TYPE_PJ_ELLPS2D_LONGITUDE_LATITUDE
    } else {
        bindings::PJ_ELLIPSOIDAL_CS_2D_TYPE_PJ_ELLPS2D_LATITUDE_LONGITUDE
    };
    let unit_name = CString::new(unit_name.as_bytes()).ok()?;
    // SAFETY: strings valid; PROJ returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_ellipsoidal_2D_cs(
            ctx.as_ptr(),
            type_,
            unit_name.as_ptr(),
            unit_conv_factor,
        )
    };
    owned(ctx, ptr)
}

/// Owned 2D cartesian coordinate system (easting/northing or north/east).
pub(crate) fn create_cartesian_2d_cs(
    ctx: &Context,
    easting_northing: bool,
    unit_name: &str,
    unit_conv_factor: f64,
) -> Option<ProjObj> {
    let type_ = if easting_northing {
        bindings::PJ_CARTESIAN_CS_2D_TYPE_PJ_CART2D_EASTING_NORTHING
    } else {
        bindings::PJ_CARTESIAN_CS_2D_TYPE_PJ_CART2D_NORTHING_EASTING
    };
    let unit_name = CString::new(unit_name.as_bytes()).ok()?;
    // SAFETY: strings valid; PROJ returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_cartesian_2D_cs(
            ctx.as_ptr(),
            type_,
            unit_name.as_ptr(),
            unit_conv_factor,
        )
    };
    owned(ctx, ptr)
}

/// Owned 3D ellipsoidal (geographic) coordinate system.
pub(crate) fn create_ellipsoidal_3d_cs(
    ctx: &Context,
    longitude_latitude_height: bool,
    horizontal_unit_name: &str,
    horizontal_unit_conv_factor: f64,
    vertical_unit_name: &str,
    vertical_unit_conv_factor: f64,
) -> Option<ProjObj> {
    let type_ = if longitude_latitude_height {
        bindings::PJ_ELLIPSOIDAL_CS_3D_TYPE_PJ_ELLPS3D_LONGITUDE_LATITUDE_HEIGHT
    } else {
        bindings::PJ_ELLIPSOIDAL_CS_3D_TYPE_PJ_ELLPS3D_LATITUDE_LONGITUDE_HEIGHT
    };
    let horizontal_unit_name = CString::new(horizontal_unit_name.as_bytes()).ok()?;
    let vertical_unit_name = CString::new(vertical_unit_name.as_bytes()).ok()?;
    // SAFETY: strings valid; PROJ returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_ellipsoidal_3D_cs(
            ctx.as_ptr(),
            type_,
            horizontal_unit_name.as_ptr(),
            horizontal_unit_conv_factor,
            vertical_unit_name.as_ptr(),
            vertical_unit_conv_factor,
        )
    };
    owned(ctx, ptr)
}

/// Owned geographic CRS built inline from datum/ellipsoid strings.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_geographic_crs(
    ctx: &Context,
    crs_name: &str,
    datum_name: &str,
    ellipsoid_name: &str,
    semi_major_metre: f64,
    inverse_flattening: f64,
    prime_meridian_name: &str,
    prime_meridian_offset: f64,
    pm_angular_units: &str,
    pm_units_conv: f64,
) -> Option<ProjObj> {
    let crs_name = CString::new(crs_name.as_bytes()).ok()?;
    let datum_name = CString::new(datum_name.as_bytes()).ok()?;
    let ellipsoid_name = CString::new(ellipsoid_name.as_bytes()).ok()?;
    let prime_meridian_name = CString::new(prime_meridian_name.as_bytes()).ok()?;
    let pm_angular_units = CString::new(pm_angular_units.as_bytes()).ok()?;
    let cs = create_ellipsoidal_2d_cs(ctx, true, "degree", 0.0174532925199433)?;
    // SAFETY: strings and cs live for the call; PROJ returns owned PJ or null
    // (the `ellipsoidal_cs` argument is *not* consumed — see PROJ docs).
    let ptr = unsafe {
        bindings::proj_create_geographic_crs(
            ctx.as_ptr(),
            crs_name.as_ptr(),
            datum_name.as_ptr(),
            ellipsoid_name.as_ptr(),
            semi_major_metre,
            inverse_flattening,
            prime_meridian_name.as_ptr(),
            prime_meridian_offset,
            pm_angular_units.as_ptr(),
            pm_units_conv,
            cs.as_ptr(),
        )
    };
    drop(cs);
    owned(ctx, ptr)
}

/// Owned geographic CRS assembled from an explicit datum + coordinate system.
///
/// `datum_or_datum_ensemble` and `ellipsoidal_cs` are only *read* by PROJ.
pub(crate) fn create_geographic_crs_from_datum(
    ctx: &Context,
    crs_name: &str,
    datum_or_datum_ensemble: &ProjObj,
    ellipsoidal_cs: &ProjObj,
) -> Option<ProjObj> {
    let crs_name = CString::new(crs_name.as_bytes()).ok()?;
    // SAFETY: all pointers live for the call; returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_geographic_crs_from_datum(
            ctx.as_ptr(),
            crs_name.as_ptr(),
            datum_or_datum_ensemble.as_ptr(),
            ellipsoidal_cs.as_ptr(),
        )
    };
    owned(ctx, ptr)
}

/// Owned projected CRS from geodetic CRS + conversion + coordinate system.
///
/// `geodetic_crs`, `conversion`, and `coordinate_system` are only *read*.
pub(crate) fn create_projected_crs(
    ctx: &Context,
    crs_name: &str,
    geodetic_crs: &ProjObj,
    conversion: &ProjObj,
    coordinate_system: &ProjObj,
) -> Option<ProjObj> {
    let crs_name = CString::new(crs_name.as_bytes()).ok()?;
    // SAFETY: all pointers live for the call; returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_projected_crs(
            ctx.as_ptr(),
            crs_name.as_ptr(),
            geodetic_crs.as_ptr(),
            conversion.as_ptr(),
            coordinate_system.as_ptr(),
        )
    };
    owned(ctx, ptr)
}

/// Owned vertical CRS built inline from datum/units strings.
pub(crate) fn create_vertical_crs(
    ctx: &Context,
    crs_name: &str,
    datum_name: &str,
    linear_units: &str,
    linear_units_conv: f64,
) -> Option<ProjObj> {
    let crs_name = CString::new(crs_name.as_bytes()).ok()?;
    let datum_name = CString::new(datum_name.as_bytes()).ok()?;
    let linear_units = CString::new(linear_units.as_bytes()).ok()?;
    // SAFETY: strings valid; PROJ returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_vertical_crs(
            ctx.as_ptr(),
            crs_name.as_ptr(),
            datum_name.as_ptr(),
            linear_units.as_ptr(),
            linear_units_conv,
        )
    };
    owned(ctx, ptr)
}

/// Owned compound (horizontal + vertical) CRS.
///
/// `horiz_crs` and `vert_crs` are only *read*.
pub(crate) fn create_compound_crs(
    ctx: &Context,
    crs_name: &str,
    horiz_crs: &ProjObj,
    vert_crs: &ProjObj,
) -> Option<ProjObj> {
    let crs_name = CString::new(crs_name.as_bytes()).ok()?;
    // SAFETY: all pointers live for the call; returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_compound_crs(
            ctx.as_ptr(),
            crs_name.as_ptr(),
            horiz_crs.as_ptr(),
            vert_crs.as_ptr(),
        )
    };
    owned(ctx, ptr)
}

/// Owned engineering CRS.
pub(crate) fn create_engineering_crs(ctx: &Context, crs_name: &str) -> Option<ProjObj> {
    let crs_name = CString::new(crs_name.as_bytes()).ok()?;
    // SAFETY: string valid; PROJ returns owned PJ or null.
    let ptr = unsafe { bindings::proj_create_engineering_crs(ctx.as_ptr(), crs_name.as_ptr()) };
    owned(ctx, ptr)
}

/// Owned bound CRS (base CRS + hub CRS + transformation). All args only *read*.
pub(crate) fn crs_create_bound_crs(
    ctx: &Context,
    base_crs: &ProjObj,
    hub_crs: &ProjObj,
    transformation: &ProjObj,
) -> Option<ProjObj> {
    // SAFETY: all pointers live for the call; returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_crs_create_bound_crs(
            ctx.as_ptr(),
            base_crs.as_ptr(),
            hub_crs.as_ptr(),
            transformation.as_ptr(),
        )
    };
    owned(ctx, ptr)
}

/// Macro generating a conversion wrapper for projections whose parameter list
/// ends with the standard (angular unit, linear unit) pair. The generated
/// function is `pub(crate)`, returns `Option<ProjObj>`, and frees nothing the
/// caller passed (PROJ builds the conversion object from scalars + strings).
macro_rules! conversion_units {
    ($(#[$doc:meta])? $name:ident, $call:path, $( $arg:ident : $ty:ty ),* $(,)?) => {
        $(#[$doc])?
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn $name(
            ctx: &Context,
            $( $arg: $ty, )*
            ang_unit_name: &str,
            ang_unit_conv_factor: f64,
            linear_unit_name: &str,
            linear_unit_conv_factor: f64,
        ) -> Option<ProjObj> {
            let ang_unit_name = CString::new(ang_unit_name.as_bytes()).ok()?;
            let linear_unit_name = CString::new(linear_unit_name.as_bytes()).ok()?;
            // SAFETY: strings/scalars valid; returns owned PJ or null.
            let ptr = unsafe {
                $call(
                    ctx.as_ptr(),
                    $( $arg, )*
                    ang_unit_name.as_ptr(),
                    ang_unit_conv_factor,
                    linear_unit_name.as_ptr(),
                    linear_unit_conv_factor,
                )
            };
            owned(ctx, ptr)
        }
    };
}

conversion_units!(
    conversion_transverse_mercator,
    bindings::proj_create_conversion_transverse_mercator,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_gauss_schreiber_transverse_mercator,
    bindings::proj_create_conversion_gauss_schreiber_transverse_mercator,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_transverse_mercator_south_oriented,
    bindings::proj_create_conversion_transverse_mercator_south_oriented,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_two_point_equidistant,
    bindings::proj_create_conversion_two_point_equidistant,
    latitude_first_point: f64,
    longitude_first_point: f64,
    latitude_second_point: f64,
    longitude_secon_point: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_tunisia_mapping_grid,
    bindings::proj_create_conversion_tunisia_mapping_grid,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_tunisia_mining_grid,
    bindings::proj_create_conversion_tunisia_mining_grid,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_albers_equal_area,
    bindings::proj_create_conversion_albers_equal_area,
    latitude_false_origin: f64,
    longitude_false_origin: f64,
    latitude_first_parallel: f64,
    latitude_second_parallel: f64,
    easting_false_origin: f64,
    northing_false_origin: f64,
);
conversion_units!(
    conversion_lambert_conic_conformal_1sp,
    bindings::proj_create_conversion_lambert_conic_conformal_1sp,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_lambert_conic_conformal_1sp_variant_b,
    bindings::proj_create_conversion_lambert_conic_conformal_1sp_variant_b,
    latitude_nat_origin: f64,
    scale: f64,
    latitude_false_origin: f64,
    longitude_false_origin: f64,
    easting_false_origin: f64,
    northing_false_origin: f64,
);
conversion_units!(
    conversion_lambert_conic_conformal_2sp,
    bindings::proj_create_conversion_lambert_conic_conformal_2sp,
    latitude_false_origin: f64,
    longitude_false_origin: f64,
    latitude_first_parallel: f64,
    latitude_second_parallel: f64,
    easting_false_origin: f64,
    northing_false_origin: f64,
);
conversion_units!(
    conversion_lambert_conic_conformal_2sp_michigan,
    bindings::proj_create_conversion_lambert_conic_conformal_2sp_michigan,
    latitude_false_origin: f64,
    longitude_false_origin: f64,
    latitude_first_parallel: f64,
    latitude_second_parallel: f64,
    easting_false_origin: f64,
    northing_false_origin: f64,
    ellipsoid_scaling_factor: f64,
);
conversion_units!(
    conversion_lambert_conic_conformal_2sp_belgium,
    bindings::proj_create_conversion_lambert_conic_conformal_2sp_belgium,
    latitude_false_origin: f64,
    longitude_false_origin: f64,
    latitude_first_parallel: f64,
    latitude_second_parallel: f64,
    easting_false_origin: f64,
    northing_false_origin: f64,
);
conversion_units!(
    conversion_azimuthal_equidistant,
    bindings::proj_create_conversion_azimuthal_equidistant,
    latitude_nat_origin: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_guam_projection,
    bindings::proj_create_conversion_guam_projection,
    latitude_nat_origin: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_bonne,
    bindings::proj_create_conversion_bonne,
    latitude_nat_origin: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_lambert_cylindrical_equal_area_spherical,
    bindings::proj_create_conversion_lambert_cylindrical_equal_area_spherical,
    latitude_first_parallel: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_lambert_cylindrical_equal_area,
    bindings::proj_create_conversion_lambert_cylindrical_equal_area,
    latitude_first_parallel: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_cassini_soldner,
    bindings::proj_create_conversion_cassini_soldner,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_equidistant_conic,
    bindings::proj_create_conversion_equidistant_conic,
    center_lat: f64,
    center_long: f64,
    latitude_first_parallel: f64,
    latitude_second_parallel: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_eckert_i,
    bindings::proj_create_conversion_eckert_i,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_eckert_ii,
    bindings::proj_create_conversion_eckert_ii,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_eckert_iii,
    bindings::proj_create_conversion_eckert_iii,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_eckert_iv,
    bindings::proj_create_conversion_eckert_iv,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_eckert_v,
    bindings::proj_create_conversion_eckert_v,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_eckert_vi,
    bindings::proj_create_conversion_eckert_vi,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_equidistant_cylindrical,
    bindings::proj_create_conversion_equidistant_cylindrical,
    latitude_first_parallel: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_equidistant_cylindrical_spherical,
    bindings::proj_create_conversion_equidistant_cylindrical_spherical,
    latitude_first_parallel: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_gall,
    bindings::proj_create_conversion_gall,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_goode_homolosine,
    bindings::proj_create_conversion_goode_homolosine,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_interrupted_goode_homolosine,
    bindings::proj_create_conversion_interrupted_goode_homolosine,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_geostationary_satellite_sweep_x,
    bindings::proj_create_conversion_geostationary_satellite_sweep_x,
    center_long: f64,
    height: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_geostationary_satellite_sweep_y,
    bindings::proj_create_conversion_geostationary_satellite_sweep_y,
    center_long: f64,
    height: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_gnomonic,
    bindings::proj_create_conversion_gnomonic,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_hotine_oblique_mercator_variant_a,
    bindings::proj_create_conversion_hotine_oblique_mercator_variant_a,
    latitude_projection_centre: f64,
    longitude_projection_centre: f64,
    azimuth_initial_line: f64,
    angle_from_rectified_to_skrew_grid: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_hotine_oblique_mercator_variant_b,
    bindings::proj_create_conversion_hotine_oblique_mercator_variant_b,
    latitude_projection_centre: f64,
    longitude_projection_centre: f64,
    azimuth_initial_line: f64,
    angle_from_rectified_to_skrew_grid: f64,
    scale: f64,
    easting_projection_centre: f64,
    northing_projection_centre: f64,
);
conversion_units!(
    conversion_hotine_oblique_mercator_two_point_natural_origin,
    bindings::proj_create_conversion_hotine_oblique_mercator_two_point_natural_origin,
    latitude_projection_centre: f64,
    latitude_point1: f64,
    longitude_point1: f64,
    latitude_point2: f64,
    longitude_point2: f64,
    scale: f64,
    easting_projection_centre: f64,
    northing_projection_centre: f64,
);
conversion_units!(
    conversion_laborde_oblique_mercator,
    bindings::proj_create_conversion_laborde_oblique_mercator,
    latitude_projection_centre: f64,
    longitude_projection_centre: f64,
    azimuth_initial_line: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_international_map_world_polyconic,
    bindings::proj_create_conversion_international_map_world_polyconic,
    center_long: f64,
    latitude_first_parallel: f64,
    latitude_second_parallel: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_krovak_north_oriented,
    bindings::proj_create_conversion_krovak_north_oriented,
    latitude_projection_centre: f64,
    longitude_of_origin: f64,
    colatitude_cone_axis: f64,
    latitude_pseudo_standard_parallel: f64,
    scale_factor_pseudo_standard_parallel: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_krovak,
    bindings::proj_create_conversion_krovak,
    latitude_projection_centre: f64,
    longitude_of_origin: f64,
    colatitude_cone_axis: f64,
    latitude_pseudo_standard_parallel: f64,
    scale_factor_pseudo_standard_parallel: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_lambert_azimuthal_equal_area,
    bindings::proj_create_conversion_lambert_azimuthal_equal_area,
    latitude_nat_origin: f64,
    longitude_nat_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_miller_cylindrical,
    bindings::proj_create_conversion_miller_cylindrical,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_mercator_variant_a,
    bindings::proj_create_conversion_mercator_variant_a,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_mercator_variant_b,
    bindings::proj_create_conversion_mercator_variant_b,
    latitude_first_parallel: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_popular_visualisation_pseudo_mercator,
    bindings::proj_create_conversion_popular_visualisation_pseudo_mercator,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_mollweide,
    bindings::proj_create_conversion_mollweide,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_new_zealand_mapping_grid,
    bindings::proj_create_conversion_new_zealand_mapping_grid,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_oblique_stereographic,
    bindings::proj_create_conversion_oblique_stereographic,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_orthographic,
    bindings::proj_create_conversion_orthographic,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_american_polyconic,
    bindings::proj_create_conversion_american_polyconic,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_polar_stereographic_variant_a,
    bindings::proj_create_conversion_polar_stereographic_variant_a,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_polar_stereographic_variant_b,
    bindings::proj_create_conversion_polar_stereographic_variant_b,
    latitude_standard_parallel: f64,
    longitude_of_origin: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_robinson,
    bindings::proj_create_conversion_robinson,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_sinusoidal,
    bindings::proj_create_conversion_sinusoidal,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_stereographic,
    bindings::proj_create_conversion_stereographic,
    center_lat: f64,
    center_long: f64,
    scale: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_van_der_grinten,
    bindings::proj_create_conversion_van_der_grinten,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_wagner_i,
    bindings::proj_create_conversion_wagner_i,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_wagner_ii,
    bindings::proj_create_conversion_wagner_ii,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_wagner_iii,
    bindings::proj_create_conversion_wagner_iii,
    latitude_true_scale: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_wagner_iv,
    bindings::proj_create_conversion_wagner_iv,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_wagner_v,
    bindings::proj_create_conversion_wagner_v,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_wagner_vi,
    bindings::proj_create_conversion_wagner_vi,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_wagner_vii,
    bindings::proj_create_conversion_wagner_vii,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_quadrilateralized_spherical_cube,
    bindings::proj_create_conversion_quadrilateralized_spherical_cube,
    center_lat: f64,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_spherical_cross_track_height,
    bindings::proj_create_conversion_spherical_cross_track_height,
    peg_point_lat: f64,
    peg_point_long: f64,
    peg_point_heading: f64,
    peg_point_height: f64,
);
conversion_units!(
    conversion_equal_earth,
    bindings::proj_create_conversion_equal_earth,
    center_long: f64,
    false_easting: f64,
    false_northing: f64,
);
conversion_units!(
    conversion_vertical_perspective,
    bindings::proj_create_conversion_vertical_perspective,
    topo_origin_lat: f64,
    topo_origin_long: f64,
    topo_origin_height: f64,
    view_point_height: f64,
    false_easting: f64,
    false_northing: f64,
);

/// UTM conversion (zone, hemisphere). No unit arguments.
pub(crate) fn conversion_utm(ctx: &Context, zone: i32, north: bool) -> Option<ProjObj> {
    // SAFETY: PROJ returns an owned conversion or null.
    let ptr = unsafe { bindings::proj_create_conversion_utm(ctx.as_ptr(), zone, north as i32) };
    owned(ctx, ptr)
}

/// GRIB-convention pole rotation. Angular units only (no linear pair).
pub(crate) fn conversion_pole_rotation_grib_convention(
    ctx: &Context,
    south_pole_lat_in_unrotated_crs: f64,
    south_pole_long_in_unrotated_crs: f64,
    axis_rotation: f64,
    ang_unit_name: &str,
    ang_unit_conv_factor: f64,
) -> Option<ProjObj> {
    let ang_unit_name = CString::new(ang_unit_name.as_bytes()).ok()?;
    // SAFETY: string valid; returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_conversion_pole_rotation_grib_convention(
            ctx.as_ptr(),
            south_pole_lat_in_unrotated_crs,
            south_pole_long_in_unrotated_crs,
            axis_rotation,
            ang_unit_name.as_ptr(),
            ang_unit_conv_factor,
        )
    };
    owned(ctx, ptr)
}

/// NetCDF-CF-convention pole rotation. Angular units only.
pub(crate) fn conversion_pole_rotation_netcdf_cf_convention(
    ctx: &Context,
    grid_north_pole_latitude: f64,
    grid_north_pole_longitude: f64,
    north_pole_grid_longitude: f64,
    ang_unit_name: &str,
    ang_unit_conv_factor: f64,
) -> Option<ProjObj> {
    let ang_unit_name = CString::new(ang_unit_name.as_bytes()).ok()?;
    // SAFETY: string valid; returns owned PJ or null.
    let ptr = unsafe {
        bindings::proj_create_conversion_pole_rotation_netcdf_cf_convention(
            ctx.as_ptr(),
            grid_north_pole_latitude,
            grid_north_pole_longitude,
            north_pole_grid_longitude,
            ang_unit_name.as_ptr(),
            ang_unit_conv_factor,
        )
    };
    owned(ctx, ptr)
}
