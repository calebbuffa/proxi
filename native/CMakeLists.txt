# proxi native superbuild
#
# Builds the complete native dependency graph for proxi into a single private
# prefix:
#
#   zlib → sqlite (lib + host sqlite3) → TLS backend → libcurl → libtiff → PROJ
#
# Everything is built from pinned, checksum-verified local archives (provided
# by the Rust archive cache). No system PROJ / CURL / TIFF is assumed. The
# result is written to ${CMAKE_INSTALL_PREFIX} and a machine-readable
# proxi-native-manifest.json is generated there for build.rs to consume.
#
# Required variables (set by build.rs):
#   PROXI_ARCHIVES_DIR   cache/archives (verified tarballs/zips)
#   PROXI_SOURCES_DIR    cache/sources (versioned extracted trees)
#   PROXI_CACHE_ROOT     cache root
#   PROXI_ENABLE_NETWORK ON/OFF  (curl + TLS)
#   PROXI_ENABLE_TIFF    ON/OFF

cmake_minimum_required(VERSION 3.20)
project(proxi-superbuild C CXX)

set(CMAKE_INSTALL_PREFIX "${CMAKE_INSTALL_PREFIX}" CACHE STRING "proxi prefix")

# Use the dynamic MSVC runtime (/MD) so every bundled native library shares the
# same CRT heap as the Rust x86_64-pc-windows-msvc target. This is required for
# malloc()/free() to be valid across the PROJ <-> Rust boundary
# (e.g. `proj_string_destroy` freeing a `proj_as_wkt` result from Rust). It also
# applies to the sqlite subdirectory configured under this superbuild.
if(MSVC)
  set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
endif()
set(ARCHIVES "${PROXI_ARCHIVES_DIR}" CACHE STRING "")
set(SOURCES "${PROXI_SOURCES_DIR}" CACHE STRING "")
set(ENABLE_NETWORK "${PROXI_ENABLE_NETWORK}" CACHE BOOL "build curl+TLS" )
set(ENABLE_TIFF "${PROXI_ENABLE_TIFF}" CACHE BOOL "build libtiff" )

# sccache/ccache if build.rs found one on PATH; empty means "none available".
set(COMPILER_LAUNCHER "${PROXI_COMPILER_LAUNCHER}" CACHE STRING "compiler cache path")
if(COMPILER_LAUNCHER)
  set(CMAKE_C_COMPILER_LAUNCHER "${COMPILER_LAUNCHER}")
  set(CMAKE_CXX_COMPILER_LAUNCHER "${COMPILER_LAUNCHER}")
endif()

# zlib, curl, tiff, proj tarballs extract a single top-level dir.
function(proxi_source_dir dep_name dep_version out)
  set(base "${SOURCES}/${dep_name}-${dep_version}")
  if(NOT IS_DIRECTORY "${base}")
    message(FATAL_ERROR "superbuild: source dir for ${dep_name} missing: ${base}")
  endif()
  file(GLOB entries "${base}/*")
  list(LENGTH entries n)
  if(n EQUAL 1 AND IS_DIRECTORY "${entries}")
    set("${out}" "${entries}" PARENT_SCOPE)
  else()
    set("${out}" "${base}" PARENT_SCOPE)
  endif()
endfunction()

include(ExternalProject)

set(COMMON_ARGS
  -DCMAKE_INSTALL_PREFIX=${CMAKE_INSTALL_PREFIX}
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON
  -DBUILD_SHARED_LIBS=OFF
  -DZLIB_USE_STATIC_LIBS=ON
)
# Propagate the single-CRT choice to every ExternalProject (zlib, curl, tiff, PROJ).
if(MSVC)
  list(APPEND COMMON_ARGS "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL")
endif()
# ExternalProject_Add configures each dep in its own cmake invocation, so the
# launcher must be passed explicitly rather than relying on the cache var above.
if(COMPILER_LAUNCHER)
  list(APPEND COMMON_ARGS
    "-DCMAKE_C_COMPILER_LAUNCHER=${COMPILER_LAUNCHER}"
    "-DCMAKE_CXX_COMPILER_LAUNCHER=${COMPILER_LAUNCHER}"
  )
endif()

# ============ SQLite: host tool + target library ============
proxi_source_dir(sqlite "3450300" SQLITE_SRC)

if(CMAKE_CROSSCOMPILING)
  # Cross-compile: build sqlite3 CLI for the HOST, sqlite lib for TARGET.

  # 1. Host sqlite3 executable (runs on the build machine).
  set(HOST_SQLITE_CMAKE_ARGS
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX=${CMAKE_BINARY_DIR}/host-tools
    -DCMAKE_SYSTEM_NAME=
    -DCMAKE_TOOLCHAIN_FILE=
    -DCMAKE_C_COMPILER=
  )
  if(CMAKE_GENERATOR_PLATFORM)
    # Do not carry a target Visual Studio platform into the host build.
    if(CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "^(AMD64|x86_64)$")
      list(APPEND HOST_SQLITE_CMAKE_ARGS -DCMAKE_GENERATOR_PLATFORM=x64)
    elseif(CMAKE_HOST_SYSTEM_PROCESSOR MATCHES "^(x86|i[3-6]86)$")
      list(APPEND HOST_SQLITE_CMAKE_ARGS -DCMAKE_GENERATOR_PLATFORM=Win32)
    endif()
  endif()
  ExternalProject_Add(sqlite3_host
    SOURCE_DIR "${SQLITE_SRC}"
    CMAKE_ARGS ${HOST_SQLITE_CMAKE_ARGS}
    BUILD_COMMAND ${CMAKE_COMMAND} --build <BINARY_DIR> --config Release --target sqlite3_exe
    INSTALL_COMMAND ${CMAKE_COMMAND} --install <BINARY_DIR> --config Release
  )
  set(SQLITE3_EXECUTABLE "${CMAKE_BINARY_DIR}/host-tools/bin/sqlite3${CMAKE_EXECUTABLE_SUFFIX}")

  # 2. Target sqlite library (compiled for the target, linked into PROJ).
  set(SQLITE_SRC_DIR "${SQLITE_SRC}" CACHE PATH "extracted sqlite" FORCE)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/sqlite" "${CMAKE_BINARY_DIR}/sqlite-build")

else()
  # Native build: one sqlite3 for everything.
  set(SQLITE_SRC_DIR "${SQLITE_SRC}" CACHE PATH "extracted sqlite" FORCE)
  add_subdirectory("${CMAKE_CURRENT_SOURCE_DIR}/sqlite" "${CMAKE_BINARY_DIR}/sqlite-build")
  set(SQLITE3_EXECUTABLE "${CMAKE_INSTALL_PREFIX}/bin/sqlite3${CMAKE_EXECUTABLE_SUFFIX}")
endif()

set(SQLITE_INSTALL "${CMAKE_INSTALL_PREFIX}")

# ---------------- zlib ----------------
proxi_source_dir(zlib "1.3.1" ZLIB_SRC)
ExternalProject_Add(ep_zlib
  SOURCE_DIR "${ZLIB_SRC}"
  CMAKE_ARGS ${COMMON_ARGS}
  INSTALL_COMMAND ${CMAKE_COMMAND} --install <BINARY_DIR>
  BUILD_BYPRODUCTS ${CMAKE_INSTALL_PREFIX}/lib/${CMAKE_STATIC_LIBRARY_PREFIX}z${CMAKE_STATIC_LIBRARY_SUFFIX}
)
set(ZLIB_INSTALL "${CMAKE_INSTALL_PREFIX}")
add_dependencies(sqlite3 ep_zlib)

# ============ OpenSSL (Linux TLS backend) ============
# Only needed on non-Windows/macOS (those use native TLS). Building it is
# required for a self-contained Linux curl. Respects CMAKE_C_COMPILER,
# CMAKE_C_FLAGS, and CMAKE_SYSTEM_PROCESSOR for architecture support.
if(NOT WIN32 AND NOT APPLE)
  proxi_source_dir(openssl "3.0.13" OPENSSL_SRC)

  # Detect OpenSSL platform string from CMAKE_SYSTEM_PROCESSOR.
  # OpenSSL uses names like "linux-x86_64", "linux-arm64", "linux-armv7l".
  set(OPENSSL_PLATFORM "linux-generic64")  # Safe fallback
  if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|AMD64)$")
    set(OPENSSL_PLATFORM "linux-x86_64")
  elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^(aarch64|arm64)$")
    set(OPENSSL_PLATFORM "linux-aarch64")
  elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^(armv7|armv7l|armv7-a)$")
    set(OPENSSL_PLATFORM "linux-armv7")
  elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^(armv6|armv6l)$")
    set(OPENSSL_PLATFORM "linux-armv6")
  elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^ppc64le")
    set(OPENSSL_PLATFORM "linux-ppc64le")
  elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^riscv64")
    # RISC-V: use generic (no explicit RISC-V target yet)
    set(OPENSSL_PLATFORM "linux-generic64")
  endif()

  # Construct CFLAGS from CMAKE_C_FLAGS + position-independent code.
  set(OPENSSL_CFLAGS "${CMAKE_C_FLAGS} -fPIC")
  find_program(OPENSSL_PERL_EXECUTABLE perl REQUIRED)
  find_program(OPENSSL_MAKE_EXECUTABLE NAMES gmake make REQUIRED)
  # OpenSSL's own build system isn't CMake, so the launcher must be baked
  # into the CC invocation directly rather than via CMAKE_C_COMPILER_LAUNCHER.
  set(OPENSSL_CC "${CMAKE_C_COMPILER}")
  if(COMPILER_LAUNCHER)
    set(OPENSSL_CC "${COMPILER_LAUNCHER} ${CMAKE_C_COMPILER}")
  endif()
  if(CMAKE_BUILD_PARALLEL_LEVEL)
    set(OPENSSL_JOBS "${CMAKE_BUILD_PARALLEL_LEVEL}")
  else()
    include(ProcessorCount)
    ProcessorCount(OPENSSL_JOBS)
    if(NOT OPENSSL_JOBS OR OPENSSL_JOBS LESS 1)
      set(OPENSSL_JOBS 1)
    endif()
  endif()

  set(OPENSSL_BUILD_TREE "${CMAKE_BINARY_DIR}/openssl-source-build")
  configure_file(
    ${CMAKE_CURRENT_SOURCE_DIR}/build-openssl.cmake.in
    ${CMAKE_BINARY_DIR}/build-openssl.cmake
    @ONLY
  )
  ExternalProject_Add(ep_openssl
    SOURCE_DIR "${OPENSSL_SRC}"
    CONFIGURE_COMMAND ""
    BUILD_COMMAND ${CMAKE_COMMAND} -P ${CMAKE_BINARY_DIR}/build-openssl.cmake
    INSTALL_COMMAND ""
    BUILD_BYPRODUCTS
      ${CMAKE_INSTALL_PREFIX}/lib/libssl${CMAKE_STATIC_LIBRARY_SUFFIX}
      ${CMAKE_INSTALL_PREFIX}/lib/libcrypto${CMAKE_STATIC_LIBRARY_SUFFIX}
  )

  message(STATUS "PROXI: Building OpenSSL for ${OPENSSL_PLATFORM} with CFLAGS=${OPENSSL_CFLAGS}")
endif()

# ---------------- libcurl ----------------
# Windows: Schannel; macOS: SecureTransport; Linux: local OpenSSL.
set(CURL_CMAKE_ARGS ${COMMON_ARGS})
list(APPEND CURL_CMAKE_ARGS
  -DCURL_DISABLE_LDAP=ON
  -DCURL_DISABLE_LDAPS=ON
  -DCURL_USE_LIBPSL=OFF
  -DCURL_USE_LIBSSH2=OFF
)
if(NOT ENABLE_NETWORK)
  set(CURL_CMAKE_ARGS ${CURL_CMAKE_ARGS} -DHTTP_ONLY=ON -DBUILD_CURL_EXE=OFF)
endif()
if(WIN32)
  list(APPEND CURL_CMAKE_ARGS -DCURL_USE_SCHANNEL=ON)
elseif(APPLE)
  list(APPEND CURL_CMAKE_ARGS -DCURL_USE_SECTRANSP=ON -DCURL_USE_OPENSSL=OFF)
else()
  list(APPEND CURL_CMAKE_ARGS -DCURL_USE_OPENSSL=ON)
endif()
proxi_source_dir(curl "8.7.1" CURL_SRC)
if(NOT WIN32 AND NOT APPLE)
  set(CURL_DEPENDS ep_zlib ep_openssl)
else()
  set(CURL_DEPENDS ep_zlib)
endif()
ExternalProject_Add(ep_curl
  SOURCE_DIR "${CURL_SRC}"
  CMAKE_ARGS ${CURL_CMAKE_ARGS}
  BUILD_BYPRODUCTS ${CMAKE_INSTALL_PREFIX}/lib/${CMAKE_STATIC_LIBRARY_PREFIX}curl${CMAKE_STATIC_LIBRARY_SUFFIX}
  DEPENDS ${CURL_DEPENDS}
)
set(CURL_INSTALL "${CMAKE_INSTALL_PREFIX}")

# ---------------- libtiff ----------------
if(ENABLE_TIFF)
  proxi_source_dir(tiff "4.6.0" TIFF_SRC)
  ExternalProject_Add(ep_tiff
    SOURCE_DIR "${TIFF_SRC}"
    CMAKE_ARGS ${COMMON_ARGS}
      -Dtiff-tools=OFF -Dtiff-tests=OFF -Dtiff-contrib=OFF -Dtiff-docs=OFF
      -Djpeg=OFF -Dwebp=OFF -Dzstd=OFF -Dlzma=OFF
      -DCMAKE_PREFIX_PATH=${CMAKE_INSTALL_PREFIX}
    BUILD_BYPRODUCTS ${CMAKE_INSTALL_PREFIX}/lib/${CMAKE_STATIC_LIBRARY_PREFIX}tiff${CMAKE_STATIC_LIBRARY_SUFFIX}
    DEPENDS ep_zlib
  )
endif()
set(TIFF_INSTALL "${CMAKE_INSTALL_PREFIX}")

# ---------------- PROJ ----------------
set(PROJ_CMAKE_ARGS ${COMMON_ARGS})
if(ENABLE_TIFF)
  list(APPEND PROJ_CMAKE_ARGS
    "-DCMAKE_PROJECT_PROJ_INCLUDE=${CMAKE_CURRENT_SOURCE_DIR}/proj-cmath-compat.cmake"
  )
endif()
list(APPEND PROJ_CMAKE_ARGS
  -DBUILD_TESTING=OFF
  -DENABLE_CURL=${ENABLE_NETWORK}
  -DENABLE_TIFF=${ENABLE_TIFF}
  -DBUILD_PROJINFO=OFF -DBUILD_PROJSYNC=OFF -DBUILD_CS2CS=OFF
  -DBUILD_GEOD=OFF -DBUILD_GIE=OFF -DBUILD_PROJ=OFF
  -DCMAKE_PREFIX_PATH=${CMAKE_INSTALL_PREFIX}
  -DSQLite3_INCLUDE_DIR=${CMAKE_INSTALL_PREFIX}/include
  -DSQLite3_LIBRARY=${CMAKE_INSTALL_PREFIX}/lib/${CMAKE_STATIC_LIBRARY_PREFIX}sqlite3${CMAKE_STATIC_LIBRARY_SUFFIX}
  -DSQLITE3_EXECUTABLE=${SQLITE3_EXECUTABLE}
)
proxi_source_dir(proj "9.4.1" PROJ_SRC)
set(PROJ_DEPENDS ep_zlib sqlite3)
if(CMAKE_CROSSCOMPILING)
  list(APPEND PROJ_DEPENDS sqlite3_host)  # Ensure host sqlite3 is available
elseif(TARGET sqlite3_exe)
  # Native build: PROJ's configure step shells out to the `sqlite3` CLI to
  # populate proj.db, so it must wait for that target too, not just the
  # library — otherwise parallel builds can race ep_proj ahead of it.
  list(APPEND PROJ_DEPENDS sqlite3_exe)
endif()
if(ENABLE_NETWORK)
  list(APPEND PROJ_DEPENDS ep_curl)
endif()
if(ENABLE_TIFF)
  list(APPEND PROJ_DEPENDS ep_tiff)
endif()
ExternalProject_Add(ep_proj
  SOURCE_DIR "${PROJ_SRC}"
  CMAKE_ARGS ${PROJ_CMAKE_ARGS}
  DEPENDS ${PROJ_DEPENDS}
  BUILD_BYPRODUCTS
    ${CMAKE_INSTALL_PREFIX}/lib/${CMAKE_STATIC_LIBRARY_PREFIX}proj${CMAKE_STATIC_LIBRARY_SUFFIX}
    ${CMAKE_INSTALL_PREFIX}/share/proj/proj.db
)

# ---------------- manifest ----------------
# Platform-specific system libraries / frameworks that PROJ + curl need but
# which are NOT part of the private prefix. These are declared here (not
# hardcoded in build.rs) so the manifest is self-contained and declarative.
set(PROXI_FRAMEWORKS_JSON "[]")
set(PROXI_SYSTEM_LIBS_JSON "[]")
if(APPLE)
  set(PROXI_FRAMEWORKS_JSON
  "[\"Security\",\"CoreFoundation\",\"CoreServices\",\"SystemConfiguration\"]")
  set(PROXI_SYSTEM_LIBS_JSON "[\"c++\"]")
elseif(WIN32)
  set(PROXI_SYSTEM_LIBS_JSON
      "[\"ws2_32\",\"crypt32\",\"secur32\",\"ncrypt\",\"wldap32\",\"schannel\",\"normaliz\",\"bcrypt\",\"ole32\",\"shell32\",\"advapi32\",\"user32\",\"gdi32\"]")
else()
  set(PROXI_SYSTEM_LIBS_JSON "[\"dl\",\"pthread\",\"m\",\"rt\",\"stdc++\"]")
endif()

if(ENABLE_NETWORK)
  set(PROXI_NETWORK_JSON "true")
else()
  set(PROXI_NETWORK_JSON "false")
endif()
if(ENABLE_TIFF)
  set(PROXI_TIFF_JSON "true")
else()
  set(PROXI_TIFF_JSON "false")
endif()

# Generate the manifest after ep_proj and all transitive dependencies have
# installed their artifacts. The script discovers actual filenames, then
# imposes a deterministic static-link order.
configure_file(
  ${CMAKE_CURRENT_SOURCE_DIR}/write-manifest.cmake.in
  ${CMAKE_BINARY_DIR}/write-manifest.cmake
  @ONLY
)
add_custom_target(proxi_manifest ALL
  COMMAND ${CMAKE_COMMAND} -P ${CMAKE_BINARY_DIR}/write-manifest.cmake
  DEPENDS ep_proj
  VERBATIM
)
