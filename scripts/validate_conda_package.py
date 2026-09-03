#!/usr/bin/env python3
"""Validate a built `.conda` package from its bytes alone.

The checks cover the archive container, the metadata in `info/`, the
consistency between `info/paths.json` and the payload, and the layout that
`scripts/build.sh` is expected to produce. Executables anywhere in the
package are parsed (ELF, Mach-O, PE) to make sure they were built for the
platform the package claims, which catches release assets that were matched
to the wrong platform.

Usage as a script:

    validate_conda_package.py [--name NAME] [--version VERSION] \\
        [--platform PLATFORM] PACKAGE.conda...

Exit status is 1 if any package has an error. Warnings never fail the run.
"""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import io
import json
import pathlib
import posixpath
import re
import struct
import sys
import tarfile
import zipfile

try:
    from compression import zstd
except ImportError:  # pragma: no cover - Python < 3.14
    print(
        "Error: validate_conda_package.py needs Python >= 3.14 for "
        "`compression.zstd`. Run it through `pixi run`.",
        file=sys.stderr,
    )
    raise

# ---------------------------------------------------------------------------
# Executable-format detection
# ---------------------------------------------------------------------------

ELF_MAGIC = b"\x7fELF"
PE_MAGIC = b"MZ"
MACHO_MAGIC_64 = 0xFEEDFACF
MACHO_MAGIC_32 = 0xFEEDFACE
FAT_MAGIC = 0xCAFEBABE
FAT_MAGIC_64 = 0xCAFEBABF
# Java class files share the FAT_MAGIC bytes; real fat headers have a tiny
# architecture count while class files have a version number here.
FAT_MAX_ARCHS = 32

ELF_MACHINES = {3: "x86", 40: "arm", 62: "x86_64", 183: "aarch64"}
PE_MACHINES = {0x14C: "x86", 0x1C0: "arm", 0x8664: "x86_64", 0xAA64: "aarch64"}
MACHO_CPUTYPES = {0x7: "x86", 0x01000007: "x86_64", 0xC: "arm", 0x0100000C: "aarch64"}

# Platform (conda subdir) -> (executable format, architecture)
PLATFORM_TARGETS: dict[str, tuple[str, str]] = {
    "linux-32": ("elf", "x86"),
    "linux-64": ("elf", "x86_64"),
    "linux-aarch64": ("elf", "aarch64"),
    "osx-64": ("macho", "x86_64"),
    "osx-arm64": ("macho", "aarch64"),
    "win-32": ("pe", "x86"),
    "win-64": ("pe", "x86_64"),
    "win-arm64": ("pe", "aarch64"),
}

FORMAT_NAMES = {"elf": "ELF", "macho": "Mach-O", "pe": "PE"}

# Top-level directories build.sh leaves alone; everything else is moved into
# extras/ (or exposed explicitly via OCTOCONDA_EXPOSE, which the validator
# cannot know about, so unknown directories are only a warning).
KNOWN_TOP_LEVEL_DIRS = {"bin", "etc", "extras", "include", "lib", "share", "ssl"}

WINDOWS_PROGRAM_SUFFIXES = (".exe", ".bat", ".cmd", ".com", ".ps1")

HEADER_BYTES = 4096


@dataclasses.dataclass(frozen=True)
class Executable:
    """Result of sniffing a file header."""

    fmt: str  # "elf", "macho", "pe", "script"
    archs: frozenset[str]  # empty for scripts / unknown machine ids

    def describe(self) -> str:
        if self.fmt == "script":
            return "script"
        archs = "/".join(sorted(self.archs)) or "unknown-arch"
        return f"{FORMAT_NAMES[self.fmt]} {archs}"


def sniff_executable(head: bytes) -> Executable | None:
    """Classify the first bytes of a file.

    Returns None for anything that is not an ELF, Mach-O or PE image and
    does not start with a shebang.
    """
    if head.startswith(ELF_MAGIC) and len(head) >= 20:
        little_endian = head[5] == 1
        machine = struct.unpack("<H" if little_endian else ">H", head[18:20])[0]
        arch = ELF_MACHINES.get(machine)
        return Executable("elf", frozenset({arch}) if arch else frozenset())

    if head.startswith(PE_MAGIC) and len(head) >= 0x40:
        pe_offset = struct.unpack("<I", head[0x3C:0x40])[0]
        if pe_offset + 6 <= len(head) and head[pe_offset : pe_offset + 4] == b"PE\0\0":
            machine = struct.unpack("<H", head[pe_offset + 4 : pe_offset + 6])[0]
            arch = PE_MACHINES.get(machine)
            return Executable("pe", frozenset({arch}) if arch else frozenset())
        return None

    if len(head) >= 8:
        magic_be = struct.unpack(">I", head[:4])[0]
        magic_le = struct.unpack("<I", head[:4])[0]
        if magic_le in (MACHO_MAGIC_64, MACHO_MAGIC_32):
            cputype = struct.unpack("<I", head[4:8])[0]
            arch = MACHO_CPUTYPES.get(cputype)
            return Executable("macho", frozenset({arch}) if arch else frozenset())
        if magic_be in (MACHO_MAGIC_64, MACHO_MAGIC_32):
            cputype = struct.unpack(">I", head[4:8])[0]
            arch = MACHO_CPUTYPES.get(cputype)
            return Executable("macho", frozenset({arch}) if arch else frozenset())
        if magic_be in (FAT_MAGIC, FAT_MAGIC_64):
            nfat = struct.unpack(">I", head[4:8])[0]
            if 0 < nfat <= FAT_MAX_ARCHS:
                entry_size = 32 if magic_be == FAT_MAGIC_64 else 20
                archs = set()
                for i in range(nfat):
                    off = 8 + i * entry_size
                    if off + 4 > len(head):
                        break
                    cputype = struct.unpack(">I", head[off : off + 4])[0]
                    arch = MACHO_CPUTYPES.get(cputype)
                    if arch:
                        archs.add(arch)
                return Executable("macho", frozenset(archs))

    if head.startswith(b"#!"):
        return Executable("script", frozenset())
    return None


# ---------------------------------------------------------------------------
# Package model
# ---------------------------------------------------------------------------


@dataclasses.dataclass
class Entry:
    """One member of the package payload."""

    path: str
    is_dir: bool
    is_symlink: bool
    link_target: str
    mode: int
    size: int
    sha256: str
    head: bytes


@dataclasses.dataclass
class Report:
    """Validation outcome for one package file."""

    package: pathlib.Path
    errors: list[str] = dataclasses.field(default_factory=list)
    warnings: list[str] = dataclasses.field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.errors

    def error(self, message: str) -> None:
        self.errors.append(message)

    def warn(self, message: str) -> None:
        self.warnings.append(message)

    def format(self) -> str:
        lines = [f"{self.package}: {'OK' if self.ok else 'FAILED'}"]
        lines += [f"  error: {e}" for e in self.errors]
        lines += [f"  warning: {w}" for w in self.warnings]
        return "\n".join(lines)


def _read_tar_zst(data: bytes) -> tarfile.TarFile:
    return tarfile.open(fileobj=io.BytesIO(zstd.decompress(data)), mode="r:")


def _load_json(tar: tarfile.TarFile, name: str, report: Report):
    try:
        member = tar.getmember(name)
    except KeyError:
        report.error(f"{name} is missing from the info archive")
        return None
    fileobj = tar.extractfile(member)
    if fileobj is None:
        report.error(f"{name} is not a regular file")
        return None
    try:
        return json.load(fileobj)
    except json.JSONDecodeError as e:
        report.error(f"{name} is not valid JSON: {e}")
        return None


def _payload_entries(tar: tarfile.TarFile, report: Report) -> list[Entry]:
    entries: list[Entry] = []
    for member in tar.getmembers():
        path = posixpath.normpath(member.name)
        if path.startswith("../") or posixpath.isabs(path):
            report.error(f"payload entry escapes the package root: {member.name}")
            continue
        if member.isdir():
            entries.append(Entry(path, True, False, "", member.mode, 0, "", b""))
            continue
        if member.issym():
            entries.append(Entry(path, False, True, member.linkname, member.mode, 0, "", b""))
            continue
        if not member.isfile():
            report.error(f"payload entry {member.name} has unsupported type {member.type!r}")
            continue
        fileobj = tar.extractfile(member)
        assert fileobj is not None
        digest = hashlib.sha256()
        head = b""
        size = 0
        while chunk := fileobj.read(1 << 20):
            if not head:
                head = chunk[:HEADER_BYTES]
            digest.update(chunk)
            size += len(chunk)
        entries.append(
            Entry(path, False, False, "", member.mode, size, digest.hexdigest(), head)
        )
    return entries


# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------


def _check_container(zf: zipfile.ZipFile, report: Report) -> tuple[bytes | None, bytes | None]:
    names = zf.namelist()
    if "metadata.json" not in names:
        report.error("metadata.json is missing from the .conda container")
    else:
        try:
            meta = json.loads(zf.read("metadata.json"))
            if meta.get("conda_pkg_format_version") != 2:
                report.error(f"unexpected conda_pkg_format_version: {meta!r}")
        except json.JSONDecodeError as e:
            report.error(f"metadata.json is not valid JSON: {e}")

    info = [n for n in names if n.startswith("info-") and n.endswith(".tar.zst")]
    pkg = [n for n in names if n.startswith("pkg-") and n.endswith(".tar.zst")]
    others = sorted(set(names) - set(info) - set(pkg) - {"metadata.json"})
    if others:
        report.error(f"unexpected members in .conda container: {', '.join(others)}")
    if len(info) != 1:
        report.error(f"expected exactly one info-*.tar.zst, found {len(info)}")
    if len(pkg) != 1:
        report.error(f"expected exactly one pkg-*.tar.zst, found {len(pkg)}")
    if len(info) != 1 or len(pkg) != 1:
        return None, None
    return zf.read(info[0]), zf.read(pkg[0])


def _check_index(
    index,
    report: Report,
    expected_name: str | None,
    expected_version: str | None,
    expected_platform: str | None,
) -> str | None:
    """Validate info/index.json and return the package's subdir."""
    if not isinstance(index, dict):
        report.error("info/index.json is not a JSON object")
        return None
    for key in ("name", "version", "build", "build_number", "subdir", "depends"):
        if key not in index:
            report.error(f"info/index.json lacks required key {key!r}")
    name = index.get("name")
    version = index.get("version")
    subdir = index.get("subdir")
    if expected_name is not None and name != expected_name:
        report.error(f"package name is {name!r}, expected {expected_name!r}")
    if expected_version is not None and version != expected_version:
        report.error(f"package version is {version!r}, expected {expected_version!r}")
    if expected_platform is not None and subdir != expected_platform:
        report.error(f"package subdir is {subdir!r}, expected {expected_platform!r}")
    if isinstance(subdir, str) and subdir not in PLATFORM_TARGETS:
        report.error(f"unsupported subdir {subdir!r}")
        return None
    return subdir if isinstance(subdir, str) else None


def _check_paths_json(paths_json, entries: list[Entry], report: Report) -> None:
    if not isinstance(paths_json, dict) or not isinstance(paths_json.get("paths"), list):
        report.error("info/paths.json has no 'paths' list")
        return
    if paths_json.get("paths_version") != 1:
        report.error(f"unexpected paths_version {paths_json.get('paths_version')!r}")

    declared: dict[str, dict] = {}
    for item in paths_json["paths"]:
        path = item.get("_path") if isinstance(item, dict) else None
        if not isinstance(path, str):
            report.error(f"malformed entry in info/paths.json: {item!r}")
            continue
        if path in declared:
            report.error(f"info/paths.json lists {path} twice")
        declared[path] = item

    actual = {e.path: e for e in entries if not e.is_dir}
    for path in sorted(set(declared) - set(actual)):
        report.error(f"info/paths.json lists {path} but it is not in the payload")
    for path in sorted(set(actual) - set(declared)):
        report.error(f"payload contains {path} which info/paths.json does not list")

    for path, item in declared.items():
        entry = actual.get(path)
        if entry is None:
            continue
        path_type = item.get("path_type")
        if entry.is_symlink:
            if path_type != "softlink":
                report.error(f"{path} is a symlink but paths.json says {path_type!r}")
            continue
        if path_type != "hardlink":
            report.error(f"{path} is a regular file but paths.json says {path_type!r}")
        if item.get("size_in_bytes") != entry.size:
            report.error(
                f"{path}: size is {entry.size} bytes, paths.json says {item.get('size_in_bytes')}"
            )
        if item.get("sha256") != entry.sha256:
            report.error(f"{path}: sha256 mismatch with info/paths.json")


def _check_symlinks(entries: list[Entry], report: Report) -> None:
    files = {e.path for e in entries if not e.is_dir}
    for entry in entries:
        if not entry.is_symlink:
            continue
        if posixpath.isabs(entry.link_target):
            report.error(f"{entry.path} is a symlink to an absolute path {entry.link_target}")
            continue
        target = posixpath.normpath(posixpath.join(posixpath.dirname(entry.path), entry.link_target))
        if target.startswith("../"):
            report.error(f"{entry.path} is a symlink pointing outside the package ({entry.link_target})")
        elif target not in files and not any(f.startswith(target + "/") for f in files):
            report.error(f"{entry.path} is a dangling symlink to {entry.link_target}")


def _check_layout(entries: list[Entry], subdir: str, version: str | None, report: Report) -> None:
    files = [e for e in entries if not e.is_dir]
    if not files:
        report.error("package payload is empty")
        return

    top_level_files = sorted(e.path for e in files if "/" not in e.path)
    for path in top_level_files:
        report.error(f"loose file at the package root: {path}")

    top_level_dirs = sorted({e.path.split("/", 1)[0] for e in files if "/" in e.path})
    for d in top_level_dirs:
        if d not in KNOWN_TOP_LEVEL_DIRS:
            report.warn(f"unexpected top-level directory {d}/ (should this be in extras/?)")

    for e in files:
        parts = e.path.split("/")
        if "__MACOSX" in parts:
            report.error(f"macOS resource fork junk in package: {e.path}")
        if parts[-1] == ".DS_Store":
            report.warn(f"macOS junk file in package: {e.path}")

    bin_files = [e for e in files if e.path.startswith("bin/")]
    bin_programs = [e for e in bin_files if e.path.count("/") == 1]
    if not bin_files:
        report.error("bin/ is missing or empty; nothing would end up on PATH")
    elif not bin_programs:
        report.error("bin/ contains only subdirectories; nothing would end up on PATH")

    is_windows = subdir.startswith("win-")
    runnable = 0
    for e in bin_programs:
        name = posixpath.basename(e.path)
        if e.is_symlink:
            runnable += 1
            continue
        exe = sniff_executable(e.head)
        if is_windows:
            if name.lower().endswith(WINDOWS_PROGRAM_SUFFIXES):
                runnable += 1
            elif exe is not None and exe.fmt == "pe":
                report.warn(f"{e.path} is a PE executable without a Windows extension")
                runnable += 1
            else:
                report.warn(f"{e.path} is not a Windows program (.exe/.bat/.cmd/.com/.ps1)")
        else:
            if name.lower().endswith(".exe"):
                report.error(f"{e.path} is a Windows program in a {subdir} package")
            if exe is None:
                report.warn(f"{e.path} is neither an executable image nor a script")
                continue
            if not e.mode & 0o111:
                report.error(f"{e.path} ({exe.describe()}) has no executable bit (mode {e.mode:o})")
            runnable += 1
        if version and version in name:
            report.warn(f"{e.path} still carries the version in its name")
    if bin_programs and runnable == 0:
        report.error("bin/ contains no runnable program")

    for e in files:
        if not e.path.startswith("extras/") or e.is_symlink:
            continue
        exe = sniff_executable(e.head)
        if exe is not None and exe.fmt != "script":
            report.warn(f"{e.path} is an executable image outside bin/")


def _check_architectures(entries: list[Entry], subdir: str, report: Report) -> None:
    want_fmt, want_arch = PLATFORM_TARGETS[subdir]
    for e in entries:
        if e.is_dir or e.is_symlink:
            continue
        exe = sniff_executable(e.head)
        if exe is None or exe.fmt == "script":
            continue
        strict = e.path.startswith(("bin/", "lib/"))
        emit = report.error if strict else report.warn
        if exe.fmt != want_fmt:
            emit(
                f"{e.path} is a {exe.describe()} image, but {subdir} needs "
                f"{FORMAT_NAMES[want_fmt]} {want_arch}"
            )
        elif exe.archs and want_arch not in exe.archs:
            emit(
                f"{e.path} is built for {'/'.join(sorted(exe.archs))}, but {subdir} needs {want_arch}"
            )
        elif not exe.archs:
            report.warn(f"{e.path} is a {FORMAT_NAMES[exe.fmt]} image with an unknown machine type")


def validate_conda_package(
    package: pathlib.Path,
    *,
    expected_name: str | None = None,
    expected_version: str | None = None,
    expected_platform: str | None = None,
) -> Report:
    """Validate one `.conda` file and return a Report of errors and warnings."""
    report = Report(package)
    if not package.is_file():
        report.error("file does not exist")
        return report
    if package.stat().st_size == 0:
        report.error("file is empty")
        return report
    try:
        zf = zipfile.ZipFile(package)
    except zipfile.BadZipFile as e:
        report.error(f"not a zip container: {e}")
        return report

    with zf:
        info_bytes, pkg_bytes = _check_container(zf, report)
    if info_bytes is None or pkg_bytes is None:
        return report

    try:
        info_tar = _read_tar_zst(info_bytes)
        pkg_tar = _read_tar_zst(pkg_bytes)
    except (zstd.ZstdError, tarfile.TarError) as e:
        report.error(f"cannot decompress payload: {e}")
        return report

    with info_tar, pkg_tar:
        index = _load_json(info_tar, "info/index.json", report)
        paths_json = _load_json(info_tar, "info/paths.json", report)
        _load_json(info_tar, "info/about.json", report)
        entries = _payload_entries(pkg_tar, report)

    subdir = _check_index(index, report, expected_name, expected_version, expected_platform)
    if paths_json is not None:
        _check_paths_json(paths_json, entries, report)
    _check_symlinks(entries, report)
    if subdir is not None:
        version = index.get("version") if isinstance(index, dict) else None
        _check_layout(entries, subdir, version if isinstance(version, str) else None, report)
        _check_architectures(entries, subdir, report)
    return report


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

PACKAGE_FILENAME_RE = re.compile(r"^(?P<name>.+)-(?P<version>[^-]+)-(?P<build>[^-]+)\.conda$")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("packages", nargs="+", type=pathlib.Path, help=".conda files to validate")
    parser.add_argument("--name", help="expected package name")
    parser.add_argument("--version", help="expected package version")
    parser.add_argument(
        "--platform",
        help="expected subdir (e.g. linux-64); defaults to the parent directory name "
        "when it looks like a subdir",
    )
    args = parser.parse_args(argv)

    failed = 0
    for package in args.packages:
        platform = args.platform
        if platform is None and package.parent.name in PLATFORM_TARGETS:
            platform = package.parent.name
        report = validate_conda_package(
            package,
            expected_name=args.name,
            expected_version=args.version,
            expected_platform=platform,
        )
        print(report.format())
        if not report.ok:
            failed += 1
    return 1 if failed else 0


# ---------------------------------------------------------------------------
# Self tests (run via `pixi run test-validate-package`)
# ---------------------------------------------------------------------------


def _fake_elf(machine: int, bits: int = 64) -> bytes:
    head = bytearray(64)
    head[0:4] = ELF_MAGIC
    head[4] = 2 if bits == 64 else 1
    head[5] = 1  # little endian
    head[16:18] = struct.pack("<H", 3)  # ET_DYN
    head[18:20] = struct.pack("<H", machine)
    return bytes(head) + b"\0" * 64


def _fake_pe(machine: int) -> bytes:
    head = bytearray(0x80)
    head[0:2] = PE_MAGIC
    head[0x3C:0x40] = struct.pack("<I", 0x40)
    head[0x40:0x44] = b"PE\0\0"
    head[0x44:0x46] = struct.pack("<H", machine)
    return bytes(head)


def _fake_macho(cputype: int) -> bytes:
    return struct.pack("<II", MACHO_MAGIC_64, cputype) + b"\0" * 56


def _fake_fat_macho(cputypes: list[int]) -> bytes:
    head = struct.pack(">II", FAT_MAGIC, len(cputypes))
    for cputype in cputypes:
        head += struct.pack(">IIIII", cputype, 0, 0, 0, 0)
    return head + b"\0" * 32


def _make_conda(
    path: pathlib.Path,
    files: dict[str, bytes | tuple[bytes, int] | str],
    *,
    name: str = "tool",
    version: str = "1.2.3",
    subdir: str = "linux-64",
    paths_override=None,
    index_override=None,
    include_about: bool = True,
) -> pathlib.Path:
    """Write a synthetic .conda file.

    `files` maps a path to its content (bytes, default mode 0o644), to a
    `(content, mode)` tuple, or to a `str` naming a symlink target.
    """

    def tar_bytes(members) -> bytes:
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w") as tar:
            for member_name, content in members:
                info = tarfile.TarInfo(member_name)
                if isinstance(content, str):
                    info.type = tarfile.SYMTYPE
                    info.linkname = content
                    tar.addfile(info)
                    continue
                data, mode = content if isinstance(content, tuple) else (content, 0o644)
                info.size = len(data)
                info.mode = mode
                tar.addfile(info, io.BytesIO(data))
        return zstd.compress(buf.getvalue())

    paths = []
    for file_name, content in files.items():
        if isinstance(content, str):
            paths.append({"_path": file_name, "path_type": "softlink"})
            continue
        data = content[0] if isinstance(content, tuple) else content
        paths.append({
            "_path": file_name,
            "path_type": "hardlink",
            "sha256": hashlib.sha256(data).hexdigest(),
            "size_in_bytes": len(data),
        })
    paths_json = {"paths": paths, "paths_version": 1}
    if paths_override is not None:
        paths_json = paths_override(paths_json)
    index = {
        "name": name, "version": version, "build": "h0_0", "build_number": 0,
        "subdir": subdir, "depends": [],
    }
    if index_override is not None:
        index = index_override(index)
    info_members = [
        ("info/index.json", json.dumps(index).encode()),
        ("info/paths.json", json.dumps(paths_json).encode()),
    ]
    if include_about:
        info_members.append(("info/about.json", b"{}"))

    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("metadata.json", json.dumps({"conda_pkg_format_version": 2}))
        zf.writestr(f"info-{name}-{version}-h0_0.tar.zst", tar_bytes(info_members))
        zf.writestr(f"pkg-{name}-{version}-h0_0.tar.zst", tar_bytes(list(files.items())))
    return path


def _validate_synthetic(tmp: str, files, **kwargs) -> Report:
    expected = {
        "expected_name": kwargs.pop("expected_name", "tool"),
        "expected_version": kwargs.pop("expected_version", "1.2.3"),
        "expected_platform": kwargs.pop("expected_platform", kwargs.get("subdir", "linux-64")),
    }
    package = _make_conda(pathlib.Path(tmp) / "tool.conda", files, **kwargs)
    return validate_conda_package(package, **expected)


def _assert_error(report: Report, needle: str) -> None:
    assert any(needle in e for e in report.errors), (needle, report.errors, report.warnings)


def _assert_warning(report: Report, needle: str) -> None:
    assert any(needle in w for w in report.warnings), (needle, report.errors, report.warnings)


def _test_sniff_executable_formats():
    assert sniff_executable(_fake_elf(62)) == Executable("elf", frozenset({"x86_64"}))
    assert sniff_executable(_fake_elf(183)) == Executable("elf", frozenset({"aarch64"}))
    assert sniff_executable(_fake_elf(3, bits=32)) == Executable("elf", frozenset({"x86"}))
    assert sniff_executable(_fake_pe(0x8664)) == Executable("pe", frozenset({"x86_64"}))
    assert sniff_executable(_fake_pe(0xAA64)) == Executable("pe", frozenset({"aarch64"}))
    assert sniff_executable(_fake_macho(0x0100000C)) == Executable("macho", frozenset({"aarch64"}))
    fat = sniff_executable(_fake_fat_macho([0x01000007, 0x0100000C]))
    assert fat == Executable("macho", frozenset({"x86_64", "aarch64"}))
    assert sniff_executable(b"#!/bin/sh\necho hi\n") == Executable("script", frozenset())
    # A Java class file shares the fat magic but has a large "arch count".
    assert sniff_executable(struct.pack(">II", FAT_MAGIC, 0x00340000) + b"\0" * 32) is None
    # MZ without a PE signature (a DOS stub only) is not a PE image.
    assert sniff_executable(b"MZ" + b"\0" * 100) is None
    assert sniff_executable(b"plain text") is None
    assert sniff_executable(b"") is None


def _test_valid_packages_per_platform():
    import tempfile
    cases = {
        "linux-64": _fake_elf(62),
        "linux-aarch64": _fake_elf(183),
        "linux-32": _fake_elf(3, bits=32),
        "osx-64": _fake_macho(0x01000007),
        "osx-arm64": _fake_fat_macho([0x01000007, 0x0100000C]),
    }
    with tempfile.TemporaryDirectory() as tmp:
        for subdir, binary in cases.items():
            report = _validate_synthetic(tmp, {"bin/tool": (binary, 0o755)}, subdir=subdir)
            assert report.ok, (subdir, report.errors)
            assert not report.warnings, (subdir, report.warnings)
        for subdir, machine in {"win-64": 0x8664, "win-arm64": 0xAA64, "win-32": 0x14C}.items():
            report = _validate_synthetic(tmp, {"bin/tool.exe": _fake_pe(machine)}, subdir=subdir)
            assert report.ok, (subdir, report.errors)
        # Scripts and symlinks in bin/ count as runnable programs.
        report = _validate_synthetic(tmp, {
            "bin/tool": (b"#!/bin/sh\nexec tool-real\n", 0o755),
            "bin/tool-real": (_fake_elf(62), 0o755),
            "bin/t": "tool",
            "share/man/man1/tool.1.gz": b"\x1f\x8b",
        })
        assert report.ok, report.errors


def _test_architecture_mismatch_is_an_error():
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        report = _validate_synthetic(tmp, {"bin/tool": (_fake_elf(183), 0o755)}, subdir="linux-64")
        _assert_error(report, "built for aarch64, but linux-64 needs x86_64")

        report = _validate_synthetic(tmp, {"bin/tool": (_fake_macho(0x01000007), 0o755)}, subdir="linux-64")
        _assert_error(report, "Mach-O x86_64 image, but linux-64 needs ELF x86_64")

        report = _validate_synthetic(tmp, {"bin/tool.exe": _fake_pe(0x14C)}, subdir="win-64")
        _assert_error(report, "built for x86, but win-64 needs x86_64")

        report = _validate_synthetic(tmp, {"bin/tool": (_fake_macho(0x01000007), 0o755)}, subdir="osx-arm64")
        _assert_error(report, "built for x86_64, but osx-arm64 needs aarch64")

        # Wrong-arch images outside bin/ and lib/ only warn.
        report = _validate_synthetic(tmp, {
            "bin/tool": (_fake_elf(62), 0o755),
            "extras/helper": _fake_elf(183),
        })
        assert report.ok, report.errors
        _assert_warning(report, "extras/helper is built for aarch64")

        report = _validate_synthetic(tmp, {
            "bin/tool": (_fake_elf(62), 0o755),
            "lib/libfoo.so": _fake_elf(183),
        })
        _assert_error(report, "lib/libfoo.so is built for aarch64")


def _test_layout_problems():
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        report = _validate_synthetic(tmp, {})
        _assert_error(report, "payload is empty")

        report = _validate_synthetic(tmp, {"tool": (_fake_elf(62), 0o755)})
        _assert_error(report, "loose file at the package root: tool")
        _assert_error(report, "bin/ is missing or empty")

        report = _validate_synthetic(tmp, {"bin/nested/tool": (_fake_elf(62), 0o755)})
        _assert_error(report, "bin/ contains only subdirectories")

        report = _validate_synthetic(tmp, {"bin/tool": (_fake_elf(62), 0o644)})
        _assert_error(report, "has no executable bit")

        report = _validate_synthetic(tmp, {"bin/tool.exe": _fake_pe(0x8664)}, subdir="linux-64")
        _assert_error(report, "Windows program in a linux-64 package")

        report = _validate_synthetic(tmp, {
            "bin/tool": (_fake_elf(62), 0o755),
            "extras/__MACOSX/._tool": b"junk",
            "extras/.DS_Store": b"junk",
            "opt/README": b"hello",
        })
        _assert_error(report, "macOS resource fork junk")
        _assert_warning(report, ".DS_Store")
        _assert_warning(report, "unexpected top-level directory opt/")

        report = _validate_synthetic(tmp, {"bin/tool-1.2.3": (_fake_elf(62), 0o755)})
        _assert_warning(report, "still carries the version in its name")

        report = _validate_synthetic(tmp, {"bin/README.md": b"docs"})
        _assert_error(report, "bin/ contains no runnable program")

        report = _validate_synthetic(tmp, {"bin/tool": (_fake_elf(62), 0o755), "bin/notes.txt": b"x"})
        _assert_warning(report, "bin/notes.txt is neither an executable image nor a script")

        report = _validate_synthetic(tmp, {"bin/tool": b"not a program"}, subdir="win-64")
        _assert_warning(report, "not a Windows program")
        _assert_error(report, "bin/ contains no runnable program")


def _test_symlink_problems():
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        report = _validate_synthetic(tmp, {"bin/tool": (_fake_elf(62), 0o755), "bin/t": "missing"})
        _assert_error(report, "dangling symlink")
        report = _validate_synthetic(tmp, {"bin/tool": (_fake_elf(62), 0o755), "bin/sh": "/bin/sh"})
        _assert_error(report, "symlink to an absolute path")
        report = _validate_synthetic(tmp, {"bin/tool": (_fake_elf(62), 0o755), "bin/up": "../../outside/file"})
        _assert_error(report, "pointing outside the package")


def _test_metadata_mismatches():
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        good = {"bin/tool": (_fake_elf(62), 0o755)}
        report = _validate_synthetic(tmp, good, expected_name="other")
        _assert_error(report, "package name is 'tool', expected 'other'")
        report = _validate_synthetic(tmp, good, expected_version="9.9")
        _assert_error(report, "package version is '1.2.3', expected '9.9'")
        report = _validate_synthetic(tmp, good, expected_platform="osx-64")
        _assert_error(report, "package subdir is 'linux-64', expected 'osx-64'")
        report = _validate_synthetic(tmp, good, include_about=False)
        _assert_error(report, "info/about.json is missing")

        def drop_depends(index):
            del index["depends"]
            return index
        report = _validate_synthetic(tmp, good, index_override=drop_depends)
        _assert_error(report, "lacks required key 'depends'")

        def corrupt_sha(paths_json):
            paths_json["paths"][0]["sha256"] = "0" * 64
            return paths_json
        report = _validate_synthetic(tmp, good, paths_override=corrupt_sha)
        _assert_error(report, "sha256 mismatch")

        def corrupt_size(paths_json):
            paths_json["paths"][0]["size_in_bytes"] += 1
            return paths_json
        report = _validate_synthetic(tmp, good, paths_override=corrupt_size)
        _assert_error(report, "size is")

        def drop_entry(paths_json):
            paths_json["paths"] = []
            return paths_json
        report = _validate_synthetic(tmp, good, paths_override=drop_entry)
        _assert_error(report, "payload contains bin/tool which info/paths.json does not list")

        def add_phantom(paths_json):
            paths_json["paths"].append({
                "_path": "bin/ghost", "path_type": "hardlink", "sha256": "0" * 64, "size_in_bytes": 1,
            })
            return paths_json
        report = _validate_synthetic(tmp, good, paths_override=add_phantom)
        _assert_error(report, "lists bin/ghost but it is not in the payload")


def _test_broken_containers():
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        missing = pathlib.Path(tmp) / "missing.conda"
        _assert_error(validate_conda_package(missing), "does not exist")

        empty = pathlib.Path(tmp) / "empty.conda"
        empty.write_bytes(b"")
        _assert_error(validate_conda_package(empty), "file is empty")

        garbage = pathlib.Path(tmp) / "garbage.conda"
        garbage.write_bytes(b"this is not a zip file")
        _assert_error(validate_conda_package(garbage), "not a zip container")

        no_payload = pathlib.Path(tmp) / "no-payload.conda"
        with zipfile.ZipFile(no_payload, "w") as zf:
            zf.writestr("metadata.json", '{"conda_pkg_format_version": 1}')
            zf.writestr("README", "hi")
        report = validate_conda_package(no_payload)
        _assert_error(report, "unexpected conda_pkg_format_version")
        _assert_error(report, "unexpected members in .conda container: README")
        _assert_error(report, "expected exactly one info-*.tar.zst, found 0")

        bad_zstd = pathlib.Path(tmp) / "bad-zstd.conda"
        with zipfile.ZipFile(bad_zstd, "w") as zf:
            zf.writestr("metadata.json", '{"conda_pkg_format_version": 2}')
            zf.writestr("info-x.tar.zst", b"not zstd")
            zf.writestr("pkg-x.tar.zst", b"not zstd")
        _assert_error(validate_conda_package(bad_zstd), "cannot decompress payload")


def _run_self_tests() -> None:
    import traceback

    failures = 0
    for test_name, fn in sorted(globals().items()):
        if test_name.startswith("_test_") and callable(fn):
            try:
                fn()
                print(f"ok {test_name}")
            except Exception:  # noqa: BLE001 - report every failing test, whatever it raised
                failures += 1
                print(f"FAIL {test_name}")
                traceback.print_exc()
    if failures:
        sys.exit(1)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--self-test":
        _run_self_tests()
    else:
        sys.exit(main())
