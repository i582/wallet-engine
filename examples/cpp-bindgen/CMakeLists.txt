cmake_minimum_required(VERSION 3.16)
project(wallet_engine_cpp_bindgen_example LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(WALLET_ENGINE_ROOT "${CMAKE_CURRENT_LIST_DIR}/../..")
set(WALLET_ENGINE_BINDINGS "${WALLET_ENGINE_ROOT}/bindings/cpp-experimental")

find_library(
    WALLET_ENGINE_LIBRARY
    NAMES wallet_engine
    PATHS "${WALLET_ENGINE_ROOT}/target/bindings-host/debug"
    NO_DEFAULT_PATH
    REQUIRED
)

add_library(
    wallet_engine_cpp_bindgen STATIC
    "${WALLET_ENGINE_BINDINGS}/wallet_engine.cpp"
)
target_include_directories(
    wallet_engine_cpp_bindgen
    PUBLIC "${WALLET_ENGINE_BINDINGS}"
)
target_link_libraries(
    wallet_engine_cpp_bindgen
    PUBLIC "${WALLET_ENGINE_LIBRARY}"
)

add_executable(wallet_engine_cpp_bindgen_example main.cpp)
target_link_libraries(
    wallet_engine_cpp_bindgen_example
    PRIVATE wallet_engine_cpp_bindgen
)

if(MSVC)
    target_compile_options(wallet_engine_cpp_bindgen_example PRIVATE /W4 /WX)
else()
    target_compile_options(
        wallet_engine_cpp_bindgen_example
        PRIVATE -Wall -Wextra -Wpedantic -Werror
    )
endif()

get_filename_component(WALLET_ENGINE_LIBRARY_DIR "${WALLET_ENGINE_LIBRARY}" DIRECTORY)
set_target_properties(
    wallet_engine_cpp_bindgen_example
    PROPERTIES BUILD_RPATH "${WALLET_ENGINE_LIBRARY_DIR}"
)
