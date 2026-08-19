# Builds SQLite amalgamation into libsqlite3 + the `sqlite3` host executable.
#
# The amalgamation is a single `sqlite3.c` (+ `sqlite3.h`, `shell.c`). This
# project compiles the library and the shell, and installs both. It is used
# by the superbuild both for the native $(target) library and (for cross
# compilation) the host `sqlite3` tool — the superbuild controls which.

cmake_minimum_required(VERSION 3.20)
project(sqlite-amalgamation C)

# Use the dynamic MSVC runtime (/MD) so this static lib shares the heap with
# Rust and the rest of the superbuild. Without this, CMake's default static
# CRT (/MT) would create a second heap, breaking cross-boundary free().
if(MSVC)
  set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
endif()

set(SQLITE_SRC_DIR "" CACHE PATH "Path to the sqlite amalgamation sources")

if(NOT SQLITE_SRC_DIR)
  set(SQLITE_SRC_DIR "${CMAKE_CURRENT_SOURCE_DIR}")
endif()

set(SQLITE_C "${SQLITE_SRC_DIR}/sqlite3.c")
set(SQLITE_H "${SQLITE_SRC_DIR}/sqlite3.h")
set(SHELL_C "${SQLITE_SRC_DIR}/shell.c")

if(NOT EXISTS "${SQLITE_C}" OR NOT EXISTS "${SQLITE_H}")
  message(FATAL_ERROR "sqlite amalgamation not found in ${SQLITE_SRC_DIR}")
endif()

# Library.
add_library(sqlite3 STATIC "${SQLITE_C}")
set_target_properties(sqlite3 PROPERTIES
  POSITION_INDEPENDENT_CODE ON
  OUTPUT_NAME "sqlite3"
)
target_include_directories(sqlite3 PUBLIC "${SQLITE_SRC_DIR}")
# Core features needed by PROJ: default + enable the FTS and R*Tree used by
# proj.db queries; keep it lean otherwise.
target_compile_definitions(sqlite3 PUBLIC
  SQLITE_ENABLE_FTS5
  SQLITE_ENABLE_RTREE
  SQLITE_THREADSAFE=1
  SQLITE_OMIT_LOAD_EXTENSION
)
if(MSVC)
  target_compile_options(sqlite3 PRIVATE /W0)
else()
  target_compile_options(sqlite3 PRIVATE -w)
endif()

# Host/CLI tool.
if(EXISTS "${SHELL_C}")
  add_executable(sqlite3_exe "${SHELL_C}")
  target_link_libraries(sqlite3_exe PRIVATE sqlite3)
  set_target_properties(sqlite3_exe PROPERTIES OUTPUT_NAME "sqlite3")
else()
  message(WARNING "sqlite shell.c not found; `sqlite3` host tool unavailable (needed by PROJ build)")
endif()

# Copy the built library, header, and CLI into the superbuild's private prefix
# (inherited via CMAKE_INSTALL_PREFIX). PROJ's FindSQLite3 needs
# prefix/include/sqlite3.h, prefix/lib/libsqlite3.a (or sqlite3.lib), and the
# `sqlite3` executable at prefix/bin/sqlite3 (or sqlite3.exe).
set(SQLITE_LIBRARY_PATH
  "${CMAKE_INSTALL_PREFIX}/lib/${CMAKE_STATIC_LIBRARY_PREFIX}sqlite3${CMAKE_STATIC_LIBRARY_SUFFIX}")
set(SQLITE_EXECUTABLE_PATH
  "${CMAKE_INSTALL_PREFIX}/bin/sqlite3${CMAKE_EXECUTABLE_SUFFIX}")
file(MAKE_DIRECTORY
  "${CMAKE_INSTALL_PREFIX}/include"
  "${CMAKE_INSTALL_PREFIX}/lib"
  "${CMAKE_INSTALL_PREFIX}/bin"
)
add_custom_command(TARGET sqlite3 POST_BUILD
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
          "${SQLITE_H}" "${CMAKE_INSTALL_PREFIX}/include/sqlite3.h"
  COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "$<TARGET_FILE:sqlite3>" "${SQLITE_LIBRARY_PATH}"
  COMMENT "Installing SQLite (lib+header) into proxi prefix"
)
if(TARGET sqlite3_exe)
  add_custom_command(TARGET sqlite3_exe POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
          "$<TARGET_FILE:sqlite3_exe>" "${SQLITE_EXECUTABLE_PATH}"
    COMMENT "Installing SQLite CLI into proxi prefix"
  )
endif()

# Install.
install(TARGETS sqlite3
  ARCHIVE DESTINATION lib
  LIBRARY DESTINATION lib
  RUNTIME DESTINATION bin
)
if(TARGET sqlite3_exe)
  install(TARGETS sqlite3_exe RUNTIME DESTINATION bin)
endif()
install(FILES "${SQLITE_H}" DESTINATION include)
