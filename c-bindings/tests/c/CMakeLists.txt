cmake_minimum_required(VERSION 3.16)
project(wallet_engine_c_tests LANGUAGES C)

include(CTest)
find_package(Threads REQUIRED)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

set(WALLET_ENGINE_ROOT "${CMAKE_CURRENT_LIST_DIR}/../../..")
find_library(
    WALLET_ENGINE_LIBRARY
    NAMES wallet_engine_c
    PATHS "${WALLET_ENGINE_ROOT}/c-bindings/target/release"
    NO_DEFAULT_PATH
    REQUIRED
)

function(add_wallet_engine_c_test target source)
    add_executable(${target} ${source})
    target_include_directories(
        ${target}
        PRIVATE "${WALLET_ENGINE_ROOT}/bindings/c"
    )
    target_link_libraries(
        ${target}
        PRIVATE "${WALLET_ENGINE_LIBRARY}" Threads::Threads
    )

    if(MSVC)
        target_compile_options(${target} PRIVATE /W4 /WX)
    else()
        target_compile_options(
            ${target}
            PRIVATE -Wall -Wextra -Wpedantic -Werror
        )
    endif()

    get_filename_component(
        WALLET_ENGINE_LIBRARY_DIR
        "${WALLET_ENGINE_LIBRARY}"
        DIRECTORY
    )
    set_target_properties(
        ${target}
        PROPERTIES BUILD_RPATH "${WALLET_ENGINE_LIBRARY_DIR}"
    )

    add_test(NAME ${target} COMMAND ${target})
endfunction()

add_wallet_engine_c_test(wallet_engine_c_abi_test abi_test.c)
add_wallet_engine_c_test(wallet_engine_c_create_wallet_test create_wallet_test.c)
