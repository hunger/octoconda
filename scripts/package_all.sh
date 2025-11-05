#!/bin/bash

set -e

test -f "build.sh" || exit 1

CURRENT="${PWD}"

echo "Uploading all conda recipes in ${CURRENT}"

for platform in "${CURRENT}/"*/; do
  # Check if it's actually a directory
  if [ -d "$platform" ]; then
    PLATFORM_DIR="${platform}"
    platform=$(basename "${PLATFORM_DIR}")
    echo "*** Processing ${platform} in ${PLATFORM_DIR}"

    for package in "${PLATFORM_DIR}/"*/; do
      PACKAGE_DIR="${package}"
      package=$(basename "${PACKAGE_DIR}")
      # Check if it's actually a directory
      if [ -d "$PACKAGE_DIR" ]; then
        echo "    * $package"
        ( cd "${PACKAGE_DIR}" && rattler-build build --target-platform="${platform}")
      fi
    done
  fi
done
