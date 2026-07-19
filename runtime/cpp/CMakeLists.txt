# The react-native track's stage-3 C++ JSI adapter (docs/tracks/react-native.md, parse-core-sdks
# repo). Two independent layers, built as separate targets so the JSI-independent core can be
# unit-tested with nothing beyond a C++17 compiler:
#
#   boltffi_jsi_core        -- pure C++ (no jsi:: type): the continuation trampoline + FfiBuf
#                              decode. Always built.
#   boltffi_jsi_core_tests  -- the pure-C++ unit tests for the above. Always built/run.
#   boltffi_jsi_adapter     -- the real jsi::HostObject-coupled adapter (rn_poc Counter/Multiplier
#                              fixture). Only built when JSI headers are available -- pass
#                              -DBOLTFFI_JSI_INCLUDE_DIR=/path/to/ReactCommon/jsi and
#                              -DBOLTFFI_CALLINVOKER_INCLUDE_DIR=/path/to/ReactCommon/callinvoker
#                              (a real `node_modules/react-native/ReactCommon` checkout, or
#                              Nitro's vendored headers -- see runtime/cpp/README.md).
#   boltffi_jsi_facebook    -- `facebook::jsi::Runtime`/`HostObject`/`Value` compiled from the
#                              REAL Meta source (jsi.cpp + jsilib-posix.cpp) found under
#                              BOLTFFI_JSI_INCLUDE_DIR/jsi/ -- proves this adapter links against
#                              the genuine JSI ABI, not a hand-rolled stub. (JSIDynamic.cpp is
#                              deliberately excluded: it is the only file in this directory that
#                              depends on folly, and this adapter never uses `folly::dynamic`.)
cmake_minimum_required(VERSION 3.16)
project(boltffi_jsi_stage3 CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_library(boltffi_jsi_core STATIC
    src/native_trampoline.cpp
    src/ffi_buf.cpp
)
target_include_directories(boltffi_jsi_core PUBLIC include)

enable_testing()

add_executable(boltffi_jsi_core_tests
    tests/native_trampoline_test.cpp
)
target_link_libraries(boltffi_jsi_core_tests PRIVATE boltffi_jsi_core)
find_package(Threads REQUIRED)
target_link_libraries(boltffi_jsi_core_tests PRIVATE Threads::Threads)
add_test(NAME boltffi_jsi_core_tests COMMAND boltffi_jsi_core_tests)

# ffi_buf's tests provide their own `boltffi_free_buf` spy (see tests/ffi_buf_test.cpp's header
# comment) -- they compile src/ffi_buf.cpp directly (for `decodeAndFreeFfiBuf`) rather than
# linking boltffi_jsi_core, which never defines `boltffi_free_buf` itself (only declares it
# extern, same as this test does) so there is no collision either way -- kept as a separate
# translation unit set for clarity about which target owns the symbol in this binary.
add_executable(boltffi_ffi_buf_tests
    src/ffi_buf.cpp
    tests/ffi_buf_test.cpp
)
target_include_directories(boltffi_ffi_buf_tests PRIVATE include)
add_test(NAME boltffi_ffi_buf_tests COMMAND boltffi_ffi_buf_tests)

if(BOLTFFI_JSI_INCLUDE_DIR)
    if(NOT EXISTS "${BOLTFFI_JSI_INCLUDE_DIR}/jsi/jsi.h")
        message(FATAL_ERROR "BOLTFFI_JSI_INCLUDE_DIR does not contain jsi/jsi.h: ${BOLTFFI_JSI_INCLUDE_DIR}")
    endif()

    add_library(boltffi_jsi_facebook STATIC
        "${BOLTFFI_JSI_INCLUDE_DIR}/jsi/jsi.cpp"
        "${BOLTFFI_JSI_INCLUDE_DIR}/jsi/jsilib-posix.cpp"
    )
    target_include_directories(boltffi_jsi_facebook PUBLIC "${BOLTFFI_JSI_INCLUDE_DIR}")
    target_compile_options(boltffi_jsi_facebook PRIVATE -Wno-unused-lambda-capture)

    add_library(boltffi_jsi_adapter STATIC
        jsi/boltffi_counter_host_object.cpp
    )
    target_include_directories(boltffi_jsi_adapter PUBLIC include)
    target_link_libraries(boltffi_jsi_adapter PUBLIC boltffi_jsi_core boltffi_jsi_facebook)
    if(BOLTFFI_CALLINVOKER_INCLUDE_DIR)
        target_include_directories(boltffi_jsi_adapter PUBLIC "${BOLTFFI_CALLINVOKER_INCLUDE_DIR}")
    endif()

    add_executable(boltffi_jsi_adapter_linkcheck
        jsi/linkcheck_main.cpp
    )
    target_link_libraries(boltffi_jsi_adapter_linkcheck PRIVATE boltffi_jsi_adapter)
    target_link_libraries(boltffi_jsi_adapter_linkcheck PRIVATE ${CMAKE_DL_LIBS})
else()
    message(STATUS "BOLTFFI_JSI_INCLUDE_DIR not set -- skipping the JSI-coupled adapter target "
                    "(boltffi_jsi_adapter/boltffi_jsi_adapter_linkcheck). The JSI-independent "
                    "core (boltffi_jsi_core) and its unit tests still build and run.")
endif()
