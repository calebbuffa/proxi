# On macOS, libtiff exports an uninstalled CMath::CMath target; libSystem
# supplies its math symbols without an extra link dependency.
if(NOT DEFINED FILE OR NOT EXISTS "${FILE}")
  message(FATAL_ERROR "patch-tiff-targets.cmake: FILE not set or missing: ${FILE}")
endif()
file(READ "${FILE}" _contents)
string(REPLACE ";CMath::CMath" "" _contents "${_contents}")
string(REPLACE "CMath::CMath;" "" _contents "${_contents}")
string(REPLACE "CMath::CMath" "" _contents "${_contents}")
file(WRITE "${FILE}" "${_contents}")
