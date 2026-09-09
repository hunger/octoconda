#!/bin/bash
set -euo pipefail

TARGET_PLATFORM="${target_platform:?}"
SOURCE="${PKG_NAME}-${PKG_VERSION}-${TARGET_PLATFORM}"
DESTINATION="${PREFIX}/share/liquibase"

mkdir -p "${DESTINATION}"
7zz x -so "${SOURCE}" > liquibase.tar
7zz x -y -snld "-o${DESTINATION}" liquibase.tar
test -f "${DESTINATION}/internal/lib/liquibase-core.jar"
find "${DESTINATION}/internal" -type f \
    \( -name 'liquibase-commercial*.jar' -o -name 'liquibase-checks.jar' \) -delete
cp "${DESTINATION}/LICENSE.txt" "${SRC_DIR}/LICENSE.txt"

case "${TARGET_PLATFORM}" in
    win-*)
        mkdir -p "${PREFIX}/Scripts"
        printf '@echo off\r\ncall "%%~dp0..\\share\\liquibase\\liquibase.bat" %%*\r\nexit /b %%ERRORLEVEL%%\r\n' \
            > "${PREFIX}/Scripts/liquibase.bat"
        ;;
    *)
        chmod 755 "${DESTINATION}/liquibase"
        mkdir -p "${PREFIX}/bin"
        ln -s ../share/liquibase/liquibase "${PREFIX}/bin/liquibase"
        ;;
esac