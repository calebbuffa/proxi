# Proxi (**PRO**J **Oxi**de)

Safe, high-performance Rust bindings for [PROJ](https://proj.org), with full
raw FFI access through `proxi::sys`.

The crate can self-provision PROJ, CURL, and TIFF from pinned,
SHA-256-verified sources into a private prefix, then reuse a local cache. It
also supports existing installations through `PROJ_DIR`, vcpkg, or pkg-config.
No prebuilt binaries are downloaded.

## Example

```rust
use proxi::{AngularUnits, Context, Coord3, TransformerBuilder};

let ctx = Context::new()?;

// Geocentric -> projected + vertical (3D).
let mut t = TransformerBuilder::new(&ctx, "EPSG:4978", "EPSG:26986+5773")
    .always_xy(true)
    .build()?;
let out = t.forward_xyz(Coord3::new(1113194.9079, -4849539.9594, 3987474.1413));

// Zero-copy SOA batch.
let mut xs = vec![6378137.0; 1024];
let mut ys = vec![0.0; 1024];
let mut zs = vec![0.0; 1024];
t.transform_xyz_in_place(&mut xs, &mut ys, &mut zs, proxi::Direction::Forward, AngularUnits::Auto)?;

// CRS inspection + database lookups.
let crs = proxi::Crs::from_authority(&ctx, "EPSG", "4326")?;
println!("{}", crs.to_wkt(proxi::WktVersion::Wkt2_2019)?);
let db = proxi::Database::new(&ctx);
for rec in db.crs_search(Some("EPSG"), &proxi::CrsSearch {
    types: vec![proxi::CrsType::ProjectedCrs],
    ..Default::default()
})? {
    println!("{} {:?}", rec.code.unwrap_or_default(), rec.name);
}
```

## Building

Provisioning order:

1. `PROXI_BUNDLED=1` forces the local superbuild.
2. `PROJ_DIR` uses an existing install with `include/proj.h` and `proj.db`.
3. vcpkg (MSVC) or pkg-config (Unix) picks up a system PROJ.
4. Otherwise the crate builds everything itself.

The superbuild compiles `zlib -> sqlite -> TLS -> libcurl -> libtiff -> PROJ`
as static libraries into a private prefix. Every dependency is pinned in
`native/versions.toml` with a checksum; archives live in a deterministic cache.

System installations are linked as provided by the platform package manager.
Their enabled PROJ features and transitive dependencies are the system
administrator's responsibility. Use `PROXI_BUNDLED=1` to require the pinned,
self-contained dependency graph.

When the bundled build is selected, the matching `proj.db` from that build is
used by default. An ambient `PROJ_DATA` from another PROJ installation is not
used to override it. This prevents a library and database version mismatch.

- Cache: `$PROXI_CACHE` (default `$CARGO_HOME/proxi-cache`).
- Offline: `PROXI_OFFLINE=1` forbids downloads and errors clearly if an
  archive is missing.

### Features

- `default = ["network", "tiff"]` — CURL + TIFF compiled in; network downloads
  are disabled until a context opts in.
- `minimal` — use `--no-default-features --features minimal` for PROJ without
  CURL/TIFF (core transforms + local `proj.db`).
- `network` — compile grid-download support.
- `tiff` — compile GeoTIFF grid support.
- `bundled` — always build from source.
- `geo`, `nalgebra`, `glam`, `serde` — ecosystem adapters (default off). The
  coordinate types implement the `Coord` trait, so you can transform them
  directly; the value types gain serde derives.

## Runtime data

The bundled build installs `proj.db` in its private prefix. `Context::new()`
creates a usable context with the build-selected PROJ data. For a configured
context, the primary database is selected in this order:

1. `ContextOptions::database_path(...)`
2. A `ContextOptions::data_paths` directory containing `proj.db`
3. The data directory selected when the linked PROJ installation was built
4. `PROJ_DATA`, when no build-selected data directory is available
5. PROJ's compiled-in system paths

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
