#!/bin/bash

set -e

test -f "build.sh" || exit 1

CURRENT="${PWD}"

echo "Uploading all conda recipes in ${CURRENT}"

RECIPE_COUNT=$(find . -type f -name recipe.yaml | wc -l) 
echo "   ${RECIPE_COUNT} recipes found"

count=0

shopt -s dotglob

for platform in "${CURRENT}/"*/; do
  # Check if it's actually a directory
  if test -d "$platform"; then
    PLATFORM_DIR="${platform}"
    platform=$(basename "${PLATFORM_DIR}")
    echo "*** Processing ${platform} in ${PLATFORM_DIR}"

    for package in "${PLATFORM_DIR}/"*/; do
      if [ "$count" -ge 100 ]; then
        echo "100 packages processed, exiting early, leaving the rest ofr later"
        exit 0
      fi

      PACKAGE_DIR="${package}"
      package=$(basename "${PACKAGE_DIR}")
      # Check if it's actually a directory
      if test -d "$PACKAGE_DIR"; then
        echo "    * ${package} (${count}/${RECIPE_COUNT})"
        if test -f "${PACKAGE_DIR}/recipe.yaml"; then
          ( cd "${PACKAGE_DIR}" && rattler-build build --target-platform="${platform}")
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
  echo "### Package build" ; \
  echo ; \
  echo "${count} of ${RECIPE_COUNT} packages processed successfully"; \
} >> status.txt

shopt -u dotglob
