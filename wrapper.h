/* proxi bindgen input.
 *
 * Includes the full stable PROJ public API surface so `scripts/bindings.sh`
 * emits *complete* bindings from the pinned PROJ version:
 *
 *   - proj.h               : core API (context/crs/operation/transform/grid/log/errno
 *                            AND the database query surface: proj_get_*_from_database,
 *                            proj_db_*, proj_deref, ...)
 *   - geodesic.h           : full geodesic library (direct/inverse/gen forms, line, polygon)
 *   - proj_experimental.h  : opt-in experimental surface
 *
 * The generated file is committed to `src/bindings.rs`; normal builds do NOT run
 * bindgen (it is a maintenance-time tool, like proj-sys/pyproj). See
 * `scripts/bindings.sh`.
 */
#include <proj.h>
#include <geodesic.h>
#include <proj_experimental.h>