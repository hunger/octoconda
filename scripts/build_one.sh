#!/bin/bash
set -e

REPO="${1:?Usage: build_one.sh <owner/repo>}"
OUTPUT_DIR="test-output"
VERIFIER="$(dirname "$(realpath "${BASH_SOURCE[0]}")")/verify_package.py"

# Wipe any artefacts from a previous run so the generator's File::create_new
# calls for build.sh / env.sh don't fail, and stale recipes from other repos
# don't leak into this build.
rm -rf "${OUTPUT_DIR:?}"
mkdir -p "${OUTPUT_DIR}"

echo "==> Generating recipes for ${REPO} into ${OUTPUT_DIR}/"
cargo run -- --filter "^${REPO}$" --work-dir "${OUTPUT_DIR}" --keep-temporary-data

# Find and build all generated recipes (without uploading)
mapfile -t RECIPES < <(find "${OUTPUT_DIR}" -type f -name recipe.yaml)
RECIPE_COUNT=${#RECIPES[@]}

if [ "${RECIPE_COUNT}" -eq 0 ]; then
  echo "No recipes generated for ${REPO}"
  exit 0
fi

echo "==> Building ${RECIPE_COUNT} recipe(s)"

SUCCESS=0
FAILED=0

for recipe in "${RECIPES[@]}"; do
  PACKAGE_DIR=$(dirname "$recipe")
  PLATFORM_DIR=$(dirname "$PACKAGE_DIR")
  package=$(basename "$PACKAGE_DIR")
  platform=$(basename "$PLATFORM_DIR")

  echo "******* ${package} [${platform}] *******"
  if python3 "${VERIFIER}" build "${recipe}" --target-platform="${platform}" --output-dir="${OUTPUT_DIR}/packages/${platform}/${package}"; then
    SUCCESS=$((SUCCESS + 1))
  else
    FAILED=$((FAILED + 1))
  fi
done

echo
echo "Done: ${SUCCESS} succeeded, ${FAILED} failed (${RECIPE_COUNT} total)"
test "${FAILED}" -eq 0
