#!/usr/bin/env bash
# Regenerate the committed FFI bindings (src/bindings.rs) from the pinned PROJ
# headers using bindgen. Run at MAINTAINER time only — when the pinned PROJ
# version or the PROJ C API changes. Normal `cargo build` never runs bindgen.
#
# The generated file is committed so consumers need no bindgen/libclang.
#
# Requirements:
#   - bindgen  (cargo install bindgen-cli)
#   - libclang (bindgen dependency)
#   - the pinned PROJ headers. They are found automatically in the local
#     superbuild cache (<PROXI_CACHE>/builds/proxi-<target>/prefix/include),
#     or via PROJ_DIR.
#
# Examples:
#   ./scripts/bindings.sh                        # uses superbuild cache
#   PROJ_DIR=/path/to/proj-install ./scripts/bindings.sh

set -euo pipefail
cd "$(dirname "$0")/.."

BINDGEN_BIN="${BINDGEN_BIN:-bindgen}"
OUT="${OUT:-src/bindings.rs}"

# Resolve the include dir: PROJ_DIR override, else the local superbuild cache.
if [[ -n "${PROJ_DIR:-}" ]]; then
  INCLUDE_DIR="${PROJ_DIR}/include"
else
  CACHE_ROOT="${PROXI_CACHE:-${CARGO_HOME:-$HOME/.cargo}/proxi-cache}"
  INCLUDE_DIR="$(find "${CACHE_ROOT}/builds" -type f -name proj.h 2>/dev/null | head -n1 | xargs -I{} dirname {})"
fi

if [[ -z "${INCLUDE_DIR}" || ! -f "${INCLUDE_DIR}/proj.h" ]]; then
  echo "ERROR: PROJ headers not found. Set PROJ_DIR to a complete installation, or run the superbuild once." >&2
  exit 1
fi

echo "Generating FULL bindings from: ${INCLUDE_DIR}"
echo "  -> ${OUT}"

# Full committed bindings: bind the complete stable + experimental + geodesic
# surface from wrapper.h (which includes proj.h, geodesic.h, proj_experimental.h),
# including the database-query surface. The output is the one committed at
# src/bindings.rs — which `proxi::sys` re-exports and to which the crate adds a
# small raw appendix (flat enum-constant aliases and the `PJ_COORD` constructors)
# in src/sys.rs.
#
# Flags (MUST match the committed bindings):
#   - default enum style "consts": `pub type PJ_* = c_int` + flat prefixed consts
#     (stable values; NO module/type shadowing). `--no-doc-comments` keeps the
#     committed file compact.
#   - PROJ/geodesic/experimental + database symbols are allowlisted; opaque
#     opaque handles stay opaque; CRT/platform types (max_align_t/time_t/FILE)
#     are blocked so no stdlib internals leak into the surface.
"${BINDGEN_BIN}" \
  --no-doc-comments \
  --blocklist-type 'max_align_t' \
  --blocklist-type 'time_t' \
  --blocklist-type 'FILE' \
  --opaque-type 'PJ_CONTEXT' \
  --opaque-type 'PJ' \
  --opaque-type 'PJ_AREA' \
  --opaque-type 'PJ_OPERATION_FACTORY_CONTEXT' \
  --opaque-type 'geod_geodesic' \
  --opaque-type 'geod_geodesicline' \
  --opaque-type 'geod_polygon' \
  --allowlist-function 'proj_.*|geod_.*|proj_context_.*|proj_create.*|proj_grid.*|proj_normalize.*|proj_area_.*|proj_as_.*|proj_angular_.*|proj_degree_.*|proj_errno.*|proj_list_.*|proj_operation_factory_context.*|proj_coordoperation.*|proj_trans.*|proj_string.*|pj_.*|proj_log_.*|proj_cleanup|proj_get_codes_from_database|proj_get_authorities_from_database|proj_get_angular_units_from_database|proj_get_linear_units_from_database|proj_get_prime_meridians_from_database|proj_get_geoid_models_from_database|proj_get_celestial_body_list|proj_db.*|proj_deref' \
  --allowlist-type 'PJ.*|PROJ.*|geod_.*' \
  --allowlist-var 'PROJ_.*|PJ_.*|GEODESIC_.*|geod_.*' \
  wrapper.h \
  -- -I"${INCLUDE_DIR}" \
  -o "${OUT}"

echo "Regenerated ${OUT}. Verify it matches the pinned PROJ version in native/versions.toml."
echo "Then confirm src/sys.rs still compiles (it adds the flat const aliases + PJ_COORD over this output)."
