# Proxi (**PRO**J **Oxi**de)

Safe, high-performance Rust bindings for [PROJ](https://proj.org), with full
raw FFI access through `proxi::sys`.

The crate can self-provision PROJ, CURL, and TIFF from pinned,
SHA-256-verified sources into a private prefix, then reuse a local cache. It
also supports existing installations through `PROJ_DIR`, vcpkg, or pkg-config.
No prebuilt binaries are downloaded.

## Example

```rust
use proxi::{AngularUnits, Atomicity, Context, Coord3, CoordBatch, Direction, TransformerBuilder};

let ctx = Context::new()?;

// Geocentric -> projected + vertical (3D).
let mut t = TransformerBuilder::new(&ctx, "EPSG:4978", "EPSG:26986+5773")
    .always_xy(true)
    .build()?;
let out = t.transform(
    Coord3::new(1113194.9079, -4849539.9594, 3987474.1413),
    Direction::Forward,
    AngularUnits::Auto,
)?;

// Many coordinates, any layout: one PROJ call for the whole slice.
let mut points = vec![Coord3::new(1113194.9079, -4849539.9594, 3987474.1413); 1024];
t.transform_coords(&mut points, Direction::Forward, AngularUnits::Auto, Atomicity::All)?;

// Zero-copy SOA batch: the buffers are handed straight to PROJ.
let mut xs = vec![6378137.0; 1024];
let mut ys = vec![0.0; 1024];
let mut zs = vec![0.0; 1024];
t.transform_batch(
    CoordBatch::new(&mut xs, &mut ys)?.with_z(&mut zs)?,
    Direction::Forward,
    AngularUnits::Auto,
    Atomicity::All,
)?;

// CRS inspection + database lookups.
let crs = proxi::Crs::from_authority(&ctx, "EPSG", "4326")?;
println!("{}", crs.to_wkt(proxi::WktVersion::Wkt2_2019, None)?);
let db = proxi::Database::new(&ctx);
for rec in db.crs_search(Some("EPSG"), &proxi::CrsSearch {
    types: vec![proxi::CrsType::ProjectedCrs],
    ..Default::default()
})? {
    println!("{} {:?}", rec.code.unwrap_or_default(), rec.name);
}
```

## API shape

There is one way to do each thing:

| Task | Method |
| --- | --- |
| One coordinate (any `Coord` type) | `transform` |
| Many coordinates in a slice (any `Coord` type, including interleaved `[f64; N]`) | `transform_coords` |
| Same, skipping `NaN`/`Inf` points | `transform_coords_finite` |
| Separate `x`/`y`/`z` buffers (fastest, zero-copy) | `transform_batch` |
| A bounds rectangle | `transform_bounds` |

Layout is not part of the method name. `transform_coords` takes any slice whose
element implements `Coord` - `Coord3`, `(f64, f64, f64)`, interleaved
`[f64; 3]`, `glam::DVec3`, `geo::Point`, or your own type - and batches the whole
slice into a single PROJ call. Reach for `transform_batch` only when your data
already lives in separate `x`/`y`/`z` columns, which is the one layout PROJ can
consume with no copy at all.

Direction and angular units are always explicit arguments rather than baked
into method names, and batch methods take an [`Atomicity`] telling them whether
a partial PROJ result may be committed to your buffers.

## Building

Provisioning order:

1. The `bundled` feature forces the local superbuild.
2. `PROJ_DIR` uses an existing install with `include/proj.h` and `proj.db`.
3. vcpkg (MSVC) or pkg-config (Unix) picks up a system PROJ.
4. Otherwise the crate builds everything itself.

The superbuild compiles `zlib -> sqlite -> TLS -> libcurl -> libtiff -> PROJ`
as static libraries into a private prefix. Every dependency is pinned in
`native/versions.toml` with a checksum; archives live in a deterministic cache.

System installations are linked as provided by the platform package manager.
Their enabled PROJ features and transitive dependencies are the system
administrator's responsibility. Enable the `bundled` feature to require the
pinned, self-contained dependency graph.

When the bundled build is selected, the matching `proj.db` from that build is
used by default. An ambient `PROJ_DATA` from another PROJ installation is not
used to override it. This prevents a library and database version mismatch.

Provisioning is handled by [`woodshed`](../woodshed), which also caches the
native build tree outside `OUT_DIR` so editing `build.rs` does not discard it.

- Cache: `$WOODSHED_CACHE` (default `$CARGO_HOME/woodshed`).
- Offline: `WOODSHED_OFFLINE=1` forbids downloads and errors clearly if an
  archive is missing.

### Features

- `default = ["network", "tiff"]` - CURL + TIFF compiled in; network downloads
  are disabled until a context opts in.
- `minimal` - use `--no-default-features --features minimal` for PROJ without
  CURL/TIFF (core transforms + local `proj.db`).
- `network` - compile grid-download support.
- `tiff` - compile GeoTIFF grid support.
- `bundled` - always build from source.
- `embedded` - implies `bundled`, embeds the bundled baseline PROJ runtime
  data into dependent binaries, and materializes it automatically for
  `Context::new()` at runtime.
- `geo`, `nalgebra`, `glam`, `serde` - ecosystem adapters (default off). The
  coordinate types implement the `Coord` trait, so you can transform them
  directly; the value types gain serde derives.

## Runtime data

The bundled build installs `proj.db` in its private prefix. With the `embedded`
feature, that baseline PROJ data is compiled into dependent binaries and
extracted automatically to a versioned user cache, so applications can ship a
single executable while still allowing grids to download at runtime. For a
configured context, the primary database is selected in this order:

1. `ContextOptions::database_path(...)`
2. A `ContextOptions::data_paths` directory containing `proj.db`
3. The materialized embedded baseline data, when built with `embedded`
4. The data directory selected when the linked PROJ installation was built
5. `PROJ_DATA`, when no build-selected data directory is available
6. PROJ's compiled-in system paths

Create an explicitly configured context with `Context::configure(...)`:

```rust
let ctx = Context::configure(
  &proxi::ContextOptions::default()
    .database_path("/opt/share/proj/proj.db")
    .user_data_dir("/var/cache/my-app/proj")
    .network_enabled(true),
)?;
```

`ContextOptions::data_paths` adds resource search directories and takes
precedence when one contains `proj.db`. `user_data_dir` is a writable search
path for downloaded grids; it does not replace the installed baseline
database. Do not point `PROJ_DATA` at a different PROJ installation when using
the bundled build.

With `.network_enabled(true)`, the user-writable download directory is created
and put on the search path so fetched grids can be downloaded and discovered.
Vertical and datum transformations may require external grids; availability
depends on the selected PROJ data and network configuration.

## Platform Notes

These details apply when the local superbuild is used. A system PROJ discovered
through `PROJ_DIR`, vcpkg, or pkg-config uses that installation's configuration.

- **Windows (MSVC):** curl uses Schannel. The static bundle uses the dynamic
  MSVC runtime, and Cargo receives the required Windows system libraries
  automatically.
- **macOS:** curl uses Secure Transport. Cargo links the required Apple
  frameworks and libc++. Set `CMAKE_OSX_ARCHITECTURES` to forward a target
  architecture to the bundled CMake build.
- **Linux:** curl uses a pinned, locally built OpenSSL. The resulting static
  link also uses the platform C++ runtime and standard system libraries.

## License

MIT or Apache-2.0, at your option.
