cmake_minimum_required(VERSION 3.21)

project(citylive_frames VERSION 1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_INCLUDE_CURRENT_DIR ON)

find_package(Qt6 6.2 REQUIRED COMPONENTS Core Gui Qml Quick)
# B61f: the row-aware texture uploads through QRhi (Qt's `rhi/qrhi.h`, shipped
# with the Gui private headers). Optional: without them the module still builds
# and falls back to a whole-image texture per frame.
find_package(Qt6 QUIET COMPONENTS GuiPrivate)

# Where plasmashell looks for the module. User-local by default: no sudo.
if(NOT DEFINED QML_INSTALL_DIR)
    set(QML_INSTALL_DIR "$ENV{HOME}/.local/lib/qt6/qml" CACHE PATH
        "Directory that QML_IMPORT_PATH points at (org/citylive/frames goes inside it)")
endif()

# One library: the backing target is the plugin itself, so there is no
# separate backing .so to install next to it.
qt_add_qml_module(cityliveframesplugin
    URI org.citylive.frames
    VERSION 1.0
    PLUGIN_TARGET cityliveframesplugin
    OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/org/citylive/frames"
    SOURCES
        cityframe.h
        cityframe.cpp
)
target_link_libraries(cityliveframesplugin PRIVATE Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick)
if(TARGET Qt6::GuiPrivate)
    target_link_libraries(cityliveframesplugin PRIVATE Qt6::GuiPrivate)
    target_compile_definitions(cityliveframesplugin PRIVATE CITYLIVE_PARTIAL_UPLOAD)
    message(STATUS "citylive frames: row-aware texture upload ON")
    # B86: many rectangles in one upload description (one staging buffer) need
    # QRhiTextureUploadDescription::setEntries(first, last). Probe for it; without
    # it each rectangle is its own upload (correct, just more bookkeeping).
    include(CheckCXXSourceCompiles)
    set(CMAKE_REQUIRED_LIBRARIES Qt6::Gui Qt6::GuiPrivate)
    set(CMAKE_REQUIRED_FLAGS "-fPIC")
    check_cxx_source_compiles("
        #include <rhi/qrhi.h>
        #include <QVector>
        int main() {
            QVector<QRhiTextureUploadEntry> e;
            QRhiTextureUploadDescription d;
            d.setEntries(e.cbegin(), e.cend());
            return 0;
        }" CITYLIVE_HAVE_SETENTRIES_RANGE)
    unset(CMAKE_REQUIRED_LIBRARIES)
    unset(CMAKE_REQUIRED_FLAGS)
    if(CITYLIVE_HAVE_SETENTRIES_RANGE)
        target_compile_definitions(cityliveframesplugin PRIVATE CITYLIVE_BATCH_UPLOAD)
        message(STATUS "citylive frames: batched rectangle upload ON")
    else()
        message(STATUS "citylive frames: setEntries(first, last) missing, one upload per rectangle")
    endif()
else()
    message(STATUS "citylive frames: Qt Gui private headers missing, whole-image upload")
endif()

install(TARGETS cityliveframesplugin
        LIBRARY DESTINATION "${QML_INSTALL_DIR}/org/citylive/frames")
install(FILES
        "${CMAKE_CURRENT_BINARY_DIR}/org/citylive/frames/qmldir"
        "${CMAKE_CURRENT_BINARY_DIR}/org/citylive/frames/cityliveframesplugin.qmltypes"
        DESTINATION "${QML_INSTALL_DIR}/org/citylive/frames")
