#!/bin/bash

# Reads CHANNEL from env.sh
set -e

test -f "env.sh" || exit 1
source "env.sh";

CURRENT="${PWD}"

echo "Uploading all conda packages in ${CURRENT} to prefix channel ${CHANNEL}"

for package in $(find "${CURRENT}" -type f -name \*.conda); do
    echo "Uploading ${package}..."
    rattler-build upload prefix -c "${CHANNEL}" ${package}
done
