cmake_minimum_required(VERSION 3.10)
project(link_sss_probe CXX)

# Path to the Link SDK. Use the copy bundled with rusty_link 0.4.9 in the
# cargo registry (the same SDK p0-probe links), so nothing is downloaded.
set(LINK_SDK "" CACHE PATH "Ableton Link SDK root (contains AbletonLinkConfig.cmake)")
if(NOT LINK_SDK)
  message(FATAL_ERROR "set -DLINK_SDK=<path to rusty_link-0.4.9/link>")
endif()

include(${LINK_SDK}/AbletonLinkConfig.cmake)

# Plain ableton::Link.
add_executable(link-sss-probe main.cpp)
target_link_libraries(link-sss-probe Ableton::Link)

# ableton::LinkAudio, as rusty_link's abl_link builds it.
add_executable(link-sss-probe-audio main.cpp)
target_compile_definitions(link-sss-probe-audio PRIVATE USE_LINK_AUDIO=1)
target_link_libraries(link-sss-probe-audio Ableton::Link)
