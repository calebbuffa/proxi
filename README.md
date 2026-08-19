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

let ctx = Context::configured()?;

// Geocentric -> projected + vertical (3D).
let mut t = TransformerBuilder::new(&ctx, "EPSG:4978", "EPSG:26986+5773")
    .always_xy(true)
    .network_enabled(true)
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
2. `PROJ_DIR` uses an existing PROJ install if it matches.
3. vcpkg (Windows) / pkg-config (Unix) picks up a system PROJ.
4. Otherwise the crate builds everything itself.

The superbuild compiles `zlib -> sqlite -> TLS -> libcurl -> libtiff -> PROJ`
into a private prefix. Every dependency is pinned in `native/versions.toml`
with a checksum; archives live in a deterministic cache.

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

`proj.db` is built into the private prefix. Data search order:

1. `TransformerBuilder::data_dir(...)` or `TransformerDefinition::data_dir(...)`
2. `ContextOptions::data_paths`
3. `PROJ_DATA`
4. bundled `prefix/share/proj`
5. system PROJ data

With `.network_enabled(true)`, the user-writable download directory is created
and put on the search path so fetched grids can be downloaded and discovered.
Vertical and datum transformations may require external grids; availability
depends on the selected PROJ data and network configuration.

## Platform notes

- **Windows (MSVC):** curl uses Schannel; the required system libs
  (`ws2_32`, `crypt32`, `secur32`, `schannel`, ...) are linked automatically.
- **macOS:** curl uses SecureTransport; build for one arch
  (`CMAKE_OSX_ARCHITECTURES`).
- **Linux:** a pinned local OpenSSL is built so the build is self-contained.

## License

MIT or Apache-2.0, at your option.
