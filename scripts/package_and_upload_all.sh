#!/bin/bash

set -e

# Reads TARGET_CHANNEL from env.sh
test -f "./env.sh" && source "./env.sh"

test -f "build.sh" || exit 1

CURRENT="${PWD}"

echo "Build and upload all conda recipes in ${CURRENT}"

RECIPE_COUNT=$(find . -type f -name recipe.yaml | wc -l) 
echo "   ${RECIPE_COUNT} recipes found"

count=0
SUCCESS_PACKAGES=0
FAILED_PACKAGES=0

shopt -s dotglob

for platform in "${CURRENT}/"*/; do
  # Check if it's actually a directory
  if test -d "$platform"; then
    PLATFORM_DIR="${platform}"
    platform=$(basename "${PLATFORM_DIR}")
    echo "*** Processing ${platform} in ${PLATFORM_DIR}"

    for package in "${PLATFORM_DIR}/"*/; do
      PACKAGE_DIR="${package}"
      package=$(basename "${PACKAGE_DIR}")
      # Check if it's actually a directory
      if test -d "$PACKAGE_DIR"; then
        echo "    * ${package} (${count}/${RECIPE_COUNT})"
        if test -f "${PACKAGE_DIR}/recipe.yaml"; then
          if ( cd "${PACKAGE_DIR}" \
              && rattler-build publish \
                  --to "https://prefix.dev/${TARGET_CHANNEL}" \
                  --build-number=+1 \
                  --generate-attestation \
                  --target-platform="${platform}" 2>&1 \
                      | grep -v 'Ignoring prefix-detection for:' \
                      | grep -v '├─ extras/' \
             ); then
            SUCCESS_PACKAGES=$((SUCCESS_PACKAGES + 1))
          else
            FAILED_PACKAGES=$((FAILED_PACKAGES + 1))
          fi
          count=$((count + 1))
        else
          echo "        NO RECIPE FOUND, SKIPPING"
        fi
      fi
    done
  fi
done

{ \
  echo ; \
  echo "## Package build" ; \
  echo ; \
  echo "Success: ${SUCCESS_PACKAGES}, Failed: ${FAILED_PACKAGES} (Total: ${count})"; \
} >> report.txt

shopt -u dotglob
