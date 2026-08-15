cmake_minimum_required(VERSION 3.16)
project(wallet_engine_cpp LANGUAGES CXX)

include(CTest)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

option(
    WALLET_ENGINE_CPP_ENABLE_SANITIZERS
    "Build Wallet Engine C++ bindings with AddressSanitizer and UndefinedBehaviorSanitizer"
    OFF
)

set(WALLET_ENGINE_ROOT "${CMAKE_CURRENT_LIST_DIR}/..")

add_library(wallet_engine_cpp STATIC src/host_bridge.cpp)
target_include_directories(
    wallet_engine_cpp
    PUBLIC "${CMAKE_CURRENT_LIST_DIR}/include"
    PRIVATE
        "${CMAKE_CURRENT_LIST_DIR}/src"
        "${WALLET_ENGINE_ROOT}/bindings/c"
)

if(MSVC)
    target_compile_options(wallet_engine_cpp PRIVATE /W4 /WX)
else()
    target_compile_options(
        wallet_engine_cpp
        PRIVATE -Wall -Wextra -Wpedantic -Werror
    )
endif()

if(WALLET_ENGINE_CPP_ENABLE_SANITIZERS)
    if(MSVC)
        message(FATAL_ERROR "The C++ sanitizer profile requires Clang or GCC")
    endif()
    target_compile_options(
        wallet_engine_cpp
        PRIVATE -fsanitize=address,undefined -fno-omit-frame-pointer
    )
    target_link_options(
        wallet_engine_cpp
        INTERFACE -fsanitize=address,undefined -fno-omit-frame-pointer
    )
endif()

if(BUILD_TESTING)
    add_executable(
        wallet_engine_cpp_host_bridge_test
        tests/host_bridge_test.cpp
    )
    target_include_directories(
        wallet_engine_cpp_host_bridge_test
        PRIVATE
            "${CMAKE_CURRENT_LIST_DIR}/src"
            "${WALLET_ENGINE_ROOT}/bindings/c"
    )
    target_link_libraries(
        wallet_engine_cpp_host_bridge_test
        PRIVATE wallet_engine_cpp
    )

    if(MSVC)
        target_compile_options(
            wallet_engine_cpp_host_bridge_test
            PRIVATE /W4 /WX
        )
    else()
        target_compile_options(
            wallet_engine_cpp_host_bridge_test
            PRIVATE -Wall -Wextra -Wpedantic -Werror
        )
    endif()

    if(WALLET_ENGINE_CPP_ENABLE_SANITIZERS)
        target_compile_options(
            wallet_engine_cpp_host_bridge_test
            PRIVATE -fsanitize=address,undefined -fno-omit-frame-pointer
        )
        target_link_options(
            wallet_engine_cpp_host_bridge_test
            PRIVATE -fsanitize=address,undefined -fno-omit-frame-pointer
        )
    endif()

    add_test(
        NAME wallet_engine_cpp_host_bridge_test
        COMMAND wallet_engine_cpp_host_bridge_test
    )
endif()
