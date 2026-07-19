# The react-native track's stage-3/4 C++ JSI adapter (docs/tracks/react-native.md, parse-core-sdks
# repo). Two independent layers, built as separate targets so the JSI-independent core can be
# unit-tested with nothing beyond a C++17 compiler:
#
#   boltffi_jsi_core        -- pure C++ (no jsi:: type): the continuation trampoline, FfiBuf decode,
#                              the ABI header parser (abi_header.*), and the generic callback-vtable
#                              builder (generic_callback.*, item B). `generic_invoke.h` (item A's
#                              call mechanism) is header-only, included by both this library's
#                              consumers and its tests directly. Always built.
#   boltffi_jsi_core_tests, boltffi_abi_header_tests, boltffi_generic_invoke_tests,
#   boltffi_generic_callback_tests, boltffi_real_crate_integration_tests
#                            -- the pure-C++ unit tests for the above. Always built/run (the last
#                              one skips itself at RUNTIME, printing why, unless
#                              BOLTFFI_RN_POC_DYLIB points at a real build of
#                              runtime/typescript/test/fixtures/rn_poc -- see that test's own doc).
#   boltffi_jsi_adapter     -- the real jsi::HostObject-coupled adapter: both the original rn_poc
#                              Counter/Multiplier fixture host object (stage 3) and the item-A/B
#                              generic host object (boltffi_generic_host_object.*, this session).
#                              Only built when JSI headers are available -- pass
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

find_package(Threads REQUIRED)

add_library(boltffi_jsi_core STATIC
    src/native_trampoline.cpp
    src/ffi_buf.cpp
    src/abi_header.cpp
    src/generic_callback.cpp
)
target_include_directories(boltffi_jsi_core PUBLIC include)
target_link_libraries(boltffi_jsi_core PUBLIC ${CMAKE_DL_LIBS} Threads::Threads)

enable_testing()

add_executable(boltffi_jsi_core_tests
    tests/native_trampoline_test.cpp
)
target_link_libraries(boltffi_jsi_core_tests PRIVATE boltffi_jsi_core)
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

# The ABI header parser's own tests read the real, committed fixture header relative to this
# source directory regardless of the invoker's CWD.
add_executable(boltffi_abi_header_tests
    tests/abi_header_test.cpp
)
target_link_libraries(boltffi_abi_header_tests PRIVATE boltffi_jsi_core)
target_compile_definitions(boltffi_abi_header_tests PRIVATE
    BOLTFFI_TEST_SOURCE_DIR="${CMAKE_CURRENT_SOURCE_DIR}")
add_test(NAME boltffi_abi_header_tests COMMAND boltffi_abi_header_tests)

# generic_invoke.h is header-only; this target just needs the include path (no library symbols).
add_executable(boltffi_generic_invoke_tests
    tests/generic_invoke_test.cpp
)
target_include_directories(boltffi_generic_invoke_tests PRIVATE include)
add_test(NAME boltffi_generic_invoke_tests COMMAND boltffi_generic_invoke_tests)

add_executable(boltffi_generic_callback_tests
    tests/generic_callback_test.cpp
)
target_link_libraries(boltffi_generic_callback_tests PRIVATE boltffi_jsi_core)
target_compile_definitions(boltffi_generic_callback_tests PRIVATE
    BOLTFFI_TEST_SOURCE_DIR="${CMAKE_CURRENT_SOURCE_DIR}")
add_test(NAME boltffi_generic_callback_tests COMMAND boltffi_generic_callback_tests)

# The real-crate proof (item A + item B against a genuine `rn_poc` build) -- always built (it needs
# no rn_poc-specific compile-time dependency, only dlopen at runtime), but skips itself at runtime
# with a clear message unless BOLTFFI_RN_POC_DYLIB is set in the test's ENVIRONMENT (not a CMake
# variable -- see runtime/typescript/test/fixtures/build-rn-poc.sh to produce it on demand; this
# repo's DISK discipline keeps the built dylib itself out of version control).
add_executable(boltffi_real_crate_integration_tests
    tests/real_crate_integration_test.cpp
)
target_link_libraries(boltffi_real_crate_integration_tests PRIVATE boltffi_jsi_core)
add_test(NAME boltffi_real_crate_integration_tests COMMAND boltffi_real_crate_integration_tests)

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
        jsi/boltffi_generic_host_object.cpp
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
