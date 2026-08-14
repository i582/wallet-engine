cmake_minimum_required(VERSION 3.16)
project(wallet_engine_c_example LANGUAGES C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

set(WALLET_ENGINE_ROOT "${CMAKE_CURRENT_LIST_DIR}/../..")
find_library(
    WALLET_ENGINE_LIBRARY
    NAMES wallet_engine_c
    PATHS "${WALLET_ENGINE_ROOT}/c-bindings/target/release"
    NO_DEFAULT_PATH
    REQUIRED
)

add_executable(wallet_engine_c_example main.c)
target_include_directories(
    wallet_engine_c_example
    PRIVATE "${WALLET_ENGINE_ROOT}/bindings/c"
)
target_link_libraries(wallet_engine_c_example PRIVATE "${WALLET_ENGINE_LIBRARY}")

get_filename_component(WALLET_ENGINE_LIBRARY_DIR "${WALLET_ENGINE_LIBRARY}" DIRECTORY)
set_target_properties(
    wallet_engine_c_example
    PROPERTIES BUILD_RPATH "${WALLET_ENGINE_LIBRARY_DIR}"
)
