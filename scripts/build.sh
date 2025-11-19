#!/bin/sh

WORK_DIR="${PWD}"

SRC="${PKG_NAME}-${PKG_VERSION}-${target_platform}"

if test -f "${SRC}.zip"; then
    ( cd "$PREFIX" && unzip -n "${WORK_DIR}/${SRC}.zip" )
elif test -f "${SRC}.tar.gz"; then
    ( cd "$PREFIX" && tar -xzf "${WORK_DIR}/${SRC}.tar.gz" )
elif test -f "${SRC}.tar.xz"; then
    ( cd "$PREFIX" && tar -xJf "${WORK_DIR}/${SRC}.tar.xz" )
elif test -f "${WORK_DIR}/${SRC}"; then
    cp "${WORK_DIR}/${SRC}" "${PREFIX}/${PKG_NAME}"
else
    echo "${SRC} not found, not a file, nor a zip not a .tar.gz"
    echo "Work directory contents is:"
    ls -alF "${WORK_DIR}"
    exit 1
fi

cd "$PREFIX" || exit 3

# Move everything out of a "foo-arch-version" folder
DIRECTORY_COUNT=$(find . -mindepth 1 -maxdepth 1 -type d -not -name conda-meta | wc -l)

if [ "$DIRECTORY_COUNT" -eq 1 ]; then
    if test -d "bin"; then
        echo "Found only a bin subdir, this looks good"
    else
        # move everything up a level
        SUBDIR=$(find . -mindepth 1 -maxdepth 1 -type d -not -name conda-meta)

        shopt -s dotglob
        mv "${SUBDIR}"/* . || true
        shopt -u dotglob
        rmdir "${SUBDIR}"
    fi
fi

# Move all executable files into bin
mkdir -p bin
mkdir -p extras

for f in *; do
    if test -f "${f}"; then
        if file "${f}" | grep "executable"; then
            chmod 755 "${f}"
        fi

        if test -x "${f}"; then
            mv "${f}" bin
        else
            case "$f" in
            *.exe|*.bat|*.com)
                mv "${f}" bin
                ;;
            *)
                mv "${f}" extras
                ;;
            esac
        fi
    elif test -d "${f}"; then
        case "${f}" in
        conda-meta|bin|etc|include|lib|man|ssl|extras)
            ;;
        *)
            mv "${f}" extras
        esac
    fi
done
