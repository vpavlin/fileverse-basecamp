#!/bin/sh
# run.sh MODE OUT.png : render fileverse_ui/Main.qml offscreen against fixtures (Qt 6.9.2 + the
# design-system sources Basecamp bundles, rev e43a481).
QD=/nix/store/agvpq5n8vcqwnkmn8bp8rlczy3fdxm6n-qtdeclarative-6.9.2
QB=/nix/store/dkfr32yi7p8cdxsnll05q1kax19fl7ay-qtbase-6.9.2
QS=/nix/store/36ndhs8khvj8960qbhvw8h3bld1kc06g-qtsvg-6.9.2
DS=${DS:-/home/vpavlin/fileverse/logos/logos-design-system/src/qml}
cd "$(dirname "$0")"
QML_XHR_ALLOW_FILE_READ=1 LANG=C.UTF-8 QT_FORCE_STDERR_LOGGING=1 QT_QPA_PLATFORM=offscreen QT_PLUGIN_PATH=$QB/lib/qt-6/plugins:$QS/lib/qt-6/plugins:$QD/lib/qt-6/plugins \
QML_IMPORT_PATH=$DS:$QD/lib/qt-6/qml timeout 60 $QD/bin/qml Harness.qml -- "$@" 2>&1 | grep -v Fontconfig
