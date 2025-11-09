#!/bin/bash

set -e

# Reads TARGET_CHANNEL from env.sh
test -f "./env.sh" && source "./env.sh"
CHANNEL="${TARGET_CHANNEL:-$1}"

test -z "${CHANNEL}" && exit 5

CURRENT="${PWD}"

echo "Uploading all conda packages in ${CURRENT} to prefix channel ${CHANNEL}"

mapfile -t -d '' files_to_process < <(find . -name \*.conda -type f -print0)
FILES_FOUND="${#files_to_process[@]}"
# FILES_FOUND=$(find . -name \*.conda -type f -print0 | tr -c '\0' '.' | tr '\0' '\n' | wc -l)
FAILED_UPLOADS=0
CURRENT=0

echo ">>> Files to process: ${FILES_FOUND}."

set -x

for file in "${files_to_process[@]}"; do
    ((CURRENT++)) || true
    echo ">>> ${CURRENT}/${FILES_FOUND}: ${file}..."
    if rattler-build upload prefix -c "${CHANNEL}" "${file}"; then
        echo "    SUCCESS"
    else
        STATUS=$?
        echo
        echo
        echo "ERROR: Upload failed with exit code $STATUS."
        echo
        echo
        
        ((FAILED_UPLOADS++)) || true
    fi
done

echo
echo "Failed uploads: ${FAILED_UPLOADS} of ${FILES_FOUND}."

echo >> status.txt
echo "### Upload" >> status.txt
echo "Failed uploads: ${FAILED_UPLOADS} of ${FILES_FOUND}." >> status.txt

exit $FAILED_UPLOADS
