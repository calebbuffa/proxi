# Third-party notices

`proxi` is licensed under MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`).
When built with the bundled native toolchain it compiles and statically links the
software below. All of it is permissively licensed. If you redistribute a binary
that contains it, reproduce each notice. This file is informational, not legal
advice. Exact versions and checksums are pinned in `native/versions.toml`.

| Component | Version | Licence             | When linked                                                        |
| --------- | ------- | ------------------- | ------------------------------------------------------------------ |
| PROJ      | 9.4.1   | MIT                 | always                                                             |
| SQLite    | 3.45.3  | Public domain       | always                                                             |
| zlib      | 1.3.1   | zlib licence        | always                                                             |
| libtiff   | 4.6.0   | libtiff (BSD-style) | `tiff` feature (default)                                           |
| libcurl   | 8.7.1   | curl (MIT-style)    | `network` feature (default)                                        |
| OpenSSL   | 3.0.13  | Apache-2.0          | `network` on Linux (Windows uses Schannel, macOS Secure Transport) |

Copy the licence text from each project's source archive (the `COPYING` /
`LICENSE` file) into your distribution's notices.

## PROJ data

`proj.db` from the PROJ source tree is bundled (and embedded with the `embedded`
feature). It is covered by the PROJ licence and contains EPSG registry data, which
is distributed under the EPSG terms of use (https://epsg.org/terms-of-use.html).
Grid files downloaded at runtime (`network`) are separate PROJ-data files; each has
its own licence, and some restrict commercial redistribution. Do not bundle them
into a product without checking their licences
(https://proj.org/en/stable/resource_files.html).

## Reducing what you ship

Applications that never download grids can drop libcurl and OpenSSL (and with them
TLS and export-compliance considerations):

```toml
proxi = { version = "0.1", default-features = false, features = ["tiff"] }
```
