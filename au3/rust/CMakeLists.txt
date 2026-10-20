#[[
Rust static library (package `au3-rs`, lib `au3_rs`) used by the au3 C++
libraries.  Cargo is the actual build system; CMake only drives it.
]]

find_program(CARGO_EXECUTABLE cargo)
if(NOT CARGO_EXECUTABLE)
   message(FATAL_ERROR "cargo not found - Rust toolchain required for au3-rs")
endif()

set(RUST_SRC_DIR  ${CMAKE_CURRENT_SOURCE_DIR})
set(RUST_TARGET_DIR ${CMAKE_BINARY_DIR}/au3-rs-target)

if(MSVC)
   set(RUST_STATIC_LIB ${RUST_TARGET_DIR}/release/au3_rs.lib)
else()
   set(RUST_STATIC_LIB ${RUST_TARGET_DIR}/release/libau3_rs.a)
endif()

file(GLOB_RECURSE RUST_SOURCES CONFIGURE_DEPENDS ${RUST_SRC_DIR}/src/*.rs)
list(APPEND RUST_SOURCES ${RUST_SRC_DIR}/Cargo.toml)

add_custom_command(
   OUTPUT ${RUST_STATIC_LIB}
   COMMAND ${CARGO_EXECUTABLE} build --release
           --manifest-path ${RUST_SRC_DIR}/Cargo.toml
           --target-dir ${RUST_TARGET_DIR}
   DEPENDS ${RUST_SOURCES}
   WORKING_DIRECTORY ${RUST_SRC_DIR}
   USES_TERMINAL
   COMMENT "Building au3-rs (cargo build --release)"
   VERBATIM
)

add_custom_target(au3-rs-build ALL DEPENDS ${RUST_STATIC_LIB})

add_library(au3-rs STATIC IMPORTED GLOBAL)
set_target_properties(au3-rs PROPERTIES
   IMPORTED_LOCATION ${RUST_STATIC_LIB}
   INTERFACE_INCLUDE_DIRECTORIES ${RUST_SRC_DIR}/include
)
add_dependencies(au3-rs au3-rs-build)

if(WIN32)
   set_property(TARGET au3-rs PROPERTY INTERFACE_LINK_LIBRARIES
      "ws2_32;userenv;ntdll;bcrypt")
else()
   find_package(Threads REQUIRED)
   set_property(TARGET au3-rs PROPERTY INTERFACE_LINK_LIBRARIES
      "Threads::Threads;${CMAKE_DL_LIBS}")
endif()
