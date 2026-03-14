#!/bin/bash

set -e

# Reads TARGET_CHANNEL from env.sh
test -f "./env.sh" && source "./env.sh"

test -f "build.sh" || exit 1

CURRENT="${PWD}"

echo "Build and upload all conda recipes in ${CURRENT}"

# Collect all recipes and shuffle so that CI timeouts don't always
# favour the same (alphabetically first) packages.
mapfile -t RECIPES < <(find "${CURRENT}" -type f -name recipe.yaml | shuf)
RECIPE_COUNT=${#RECIPES[@]}
echo "   ${RECIPE_COUNT} recipes found (shuffled)"

count=0
SUCCESS_PACKAGES=0
FAILED_PACKAGES=0

shopt -s dotglob

for recipe in "${RECIPES[@]}"; do
  PACKAGE_DIR=$(dirname "$recipe")
  PLATFORM_DIR=$(dirname "$PACKAGE_DIR")
  package=$(basename "$PACKAGE_DIR")
  platform=$(basename "$PLATFORM_DIR")

  echo "******* ${package} [${platform}] (${count}/${RECIPE_COUNT}, ${FAILED_PACKAGES} not OK) ******"
  BUILD_OUTPUT=$(cd "${PACKAGE_DIR}" \
      && rattler-build publish \
          --to "https://prefix.dev/${TARGET_CHANNEL}" \
          --generate-attestation \
          --target-platform="${platform}" 2>&1) \
    && SUCCESS_PACKAGES=$((SUCCESS_PACKAGES + 1)) \
    || { FAILED_PACKAGES=$((FAILED_PACKAGES + 1)); echo "${BUILD_OUTPUT}"; }
  count=$((count + 1))

  # Clean up! We do not want to run out of storage
  rm -rf "${PACKAGE_DIR}" || true
done

{ \
  echo ; \
  echo "## Package build" ; \
  echo ; \
  echo "Success: ${SUCCESS_PACKAGES}, Failed: ${FAILED_PACKAGES} (Total: ${count})"; \
} >> report.txt

shopt -u dotglob
