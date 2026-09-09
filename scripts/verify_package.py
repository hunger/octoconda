#!/usr/bin/env python3
"""Verify that packaged file bytes come from upstream release assets."""

import argparse
import hashlib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch
from urllib.request import urlopen


def file_sha256(path: Path) -> str:
    """Return the SHA-256 digest of a file's contents."""
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def tree_hashes(root: Path, *, package: bool = False) -> dict[str, str]:
    """Hash files, rejecting links that escape the extracted tree."""
    root = root.resolve()
    hashes: dict[str, str] = {}
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root)
        if package and relative.parts[0] == "info" and relative.parts[1:2] != ("licenses",):
            continue
        resolved = path.resolve(strict=True)
        if not resolved.is_relative_to(root):
            raise ValueError(f"Link escapes extracted tree: {relative}")
        if path.is_file():
            hashes[relative.as_posix()] = file_sha256(path)
        elif not path.is_dir():
            raise ValueError(f"Unsupported file type: {relative}")
    return hashes


def verify_contents(
    package_dir: Path, upstream_hashes: set[str], launchers: dict[str, str] | None = None,
) -> int:
    """Require every payload and license file to match an upstream SHA-256."""
    launchers = launchers or {}
    packaged_hashes = tree_hashes(package_dir, package=True)
    if not packaged_hashes:
        raise ValueError("Package contains no payload files")
    for name, digest in launchers.items():
        if name not in packaged_hashes or packaged_hashes[name] != digest:
            raise ValueError(f"Generated launcher is missing or has a different SHA-256: {name}")
    unmatched = [
        f"{name}: SHA-256 {digest}"
        for name, digest in packaged_hashes.items()
        if digest not in upstream_hashes and name not in launchers
    ]
    if unmatched:
        raise ValueError("Files not found in upstream assets:\n" + "\n".join(unmatched))
    return len(packaged_hashes)


def unpack_source(source: Path, destination: Path) -> None:
    """Extract release archives without modifying the downloaded file."""
    destination.mkdir()
    with tempfile.TemporaryDirectory() as temporary:
        current = source.resolve()
        for step in range(5):
            file_type = subprocess.check_output(["file", "--brief", str(current)], text=True).lower()
            if any(kind in file_type for kind in ("zip archive", "tar archive", "7-zip archive")):
                subprocess.run(
                    ["7zz", "x", "-y", "-snld", f"-o{destination.resolve()}", str(current)],
                    check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                )
                return
            if any(kind in file_type for kind in (
                "gzip compressed", "xz compressed", "zstandard compressed", "zstd compressed",
                "bzip2 compressed",
            )):
                decompressed = Path(temporary) / str(step)
                with decompressed.open("wb") as stream:
                    subprocess.run(
                        ["7zz", "e", "-so", "-snld", str(current)], check=True,
                        stdout=stream, stderr=subprocess.PIPE,
                    )
                current = decompressed
            else:
                shutil.copyfile(current, destination / source.name)
                return
    raise ValueError(f"Too many compression layers: {source}")


def source_hashes(sources: list[Path], directory: Path) -> set[str]:
    """Collect SHA-256 digests from every supplied upstream distribution."""
    hashes: set[str] = set()
    for index, source in enumerate(sources):
        extracted = directory / f"source-{index}"
        hashes.add(file_sha256(source))
        unpack_source(source, extracted)
        hashes.update(tree_hashes(extracted).values())
    if not hashes:
        raise ValueError("Upstream assets contain no files")
    return hashes


def download_sources(manifest: Path, directory: Path) -> list[Path]:
    """Download generator-selected assets and check available upstream digests."""
    assets = json.loads(manifest.read_text())
    if not isinstance(assets, list) or not assets:
        raise ValueError(f"No upstream assets in {manifest}")
    sources: list[Path] = []
    for index, asset in enumerate(assets):
        url = asset["url"]
        if not isinstance(url, str) or not url.startswith("https://"):
            raise ValueError("Upstream asset URLs must use HTTPS")
        source = directory / f"download-{index}"
        with urlopen(url, timeout=60) as response, source.open("wb") as stream:
            shutil.copyfileobj(response, stream)
        expected = asset.get("sha256")
        if expected is not None and file_sha256(source) != expected:
            raise ValueError(f"Upstream SHA-256 mismatch: {url}")
        sources.append(source)
    return sources


def verify_archive(package: Path, upstream_hashes: set[str]) -> None:
    """Verify the actual Conda payload after all build-time processing."""
    with tempfile.TemporaryDirectory() as temporary:
        extracted = Path(temporary) / "package"
        subprocess.run(
            ["rattler-build", "package", "extract", str(package), "--dest", str(extracted)],
            check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        about = json.loads((extracted / "info" / "about.json").read_text())
        launchers = about.get("extra", {}).get("generated-launchers", {})
        count = verify_contents(extracted, upstream_hashes, launchers)
        print(
            f"Verified {count} file SHA-256 digests ({len(launchers)} approved launchers): "
            f"{package.name}", flush=True,
        )


def built_packages(output: Path, platform: str) -> list[Path]:
    """Find target and noarch outputs, excluding source caches and broken builds."""
    return sorted(
        package
        for subdir in {platform, "noarch"}
        for pattern in ("*.conda", "*.tar.bz2")
        for package in (output / subdir).glob(pattern)
    )


def build_verified(recipe: Path, platform: str, output: Path, publish_to: str | None) -> None:
    """Build and verify all outputs before optionally publishing those archives."""
    recipe = recipe.resolve()
    output = output.resolve()
    if built_packages(output, platform):
        raise ValueError(f"Output directory already contains packages: {output}")
    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary)
        sources = download_sources(recipe.parent / "upstream-assets.json", directory)
        hashes = source_hashes(sources, directory)
    subprocess.run(
        ["rattler-build", "build", "--recipe", str(recipe), "--target-platform", platform,
         "--output-dir", str(output)], check=True,
    )
    packages = built_packages(output, platform)
    if not packages:
        raise ValueError("Build produced no packages to verify")
    for package in packages:
        verify_archive(package, hashes)
    if publish_to:
        subprocess.run(
            ["rattler-build", "publish", "--to", publish_to, "--generate-attestation",
             *map(str, packages)], check=True,
        )


def main() -> None:
    """Run standalone verification or the verified build/publish workflow."""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify", help="Verify a built package against release assets")
    verify.add_argument("package", type=Path)
    upstream = verify.add_mutually_exclusive_group(required=True)
    upstream.add_argument("--source", type=Path, action="append", help="Local upstream asset; repeatable")
    upstream.add_argument("--sources-json", type=Path, help="Generator's upstream-assets.json")
    build = commands.add_parser("build", help="Build, verify, and optionally publish")
    build.add_argument("recipe", type=Path)
    build.add_argument("--target-platform", required=True)
    build.add_argument("--output-dir", type=Path, required=True)
    build.add_argument("--publish-to")
    arguments = parser.parse_args()
    try:
        if arguments.command == "build":
            build_verified(arguments.recipe, arguments.target_platform, arguments.output_dir,
                           arguments.publish_to)
        else:
            with tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                sources = arguments.source or download_sources(arguments.sources_json, directory)
                verify_archive(arguments.package, source_hashes(sources, directory))
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Package verification failed: {error}\n")


class VerificationTests(unittest.TestCase):
    """Exercise byte identity independently of file layout."""

    def test_renamed_files_links_and_multiple_sources(self) -> None:
        """Accept identical bytes from either source, including linked launchers."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = [root / "source-one", root / "source-two"]
            for source in sources:
                source.mkdir()
            (sources[0] / "original").write_bytes(b"upstream executable")
            (sources[1] / "LICENSE").write_bytes(b"upstream license")
            package = root / "package"
            (package / "bin").mkdir(parents=True)
            (package / "bin" / "renamed").write_bytes(b"upstream executable")
            (package / "bin" / "alias").symlink_to("renamed")
            (package / "info" / "licenses").mkdir(parents=True)
            (package / "info" / "index.json").write_text("{}")
            (package / "info" / "licenses" / "LICENSE").write_bytes(b"upstream license")
            hashes = {digest for source in sources for digest in tree_hashes(source).values()}
            self.assertEqual(verify_contents(package, hashes), 3)
            (package / "info" / "licenses" / "LICENSE").write_bytes(b"modified license")
            with self.assertRaisesRegex(ValueError, "info/licenses/LICENSE"):
                verify_contents(package, hashes)

    def test_modified_and_added_files_fail(self) -> None:
        """Reject changed files, generated wrappers, and hidden payload files."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "original").write_bytes(b"upstream executable")
            package = root / "package"
            package.mkdir()
            for name in ["original", "wrapper.bat", ".hidden"]:
                with self.subTest(name=name):
                    path = package / name
                    path.write_bytes(b"different bytes")
                    with self.assertRaisesRegex(ValueError, name):
                        verify_contents(package, set(tree_hashes(source).values()))
                    path.unlink()

    def test_external_and_broken_links_fail(self) -> None:
        """Reject links whose contents cannot be verified inside the package."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "package"
            package.mkdir()
            (root / "external").write_bytes(b"outside package")
            link = package / "link"
            link.symlink_to("../external")
            with self.assertRaisesRegex(ValueError, "escapes"):
                tree_hashes(package, package=True)
            link.unlink()
            link.symlink_to("missing")
            with self.assertRaises(FileNotFoundError):
                tree_hashes(package, package=True)

    def test_archive_and_bare_binary(self) -> None:
        """Hash both archived contents and standalone downloaded executables."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "archive-without-extension"
            with zipfile.ZipFile(archive, "w") as zipped:
                zipped.writestr("nested/file", b"archived bytes")
            binary = root / "binary"
            binary.write_bytes(b"standalone bytes")
            hashes = source_hashes([archive, binary], root)
            self.assertEqual(hashes, {
                hashlib.sha256(b"archived bytes").hexdigest(), file_sha256(binary),
                file_sha256(archive),
            })

    def test_launcher_exception_requires_exact_path_and_bytes(self) -> None:
        """Allow only an explicitly declared launcher with unchanged bytes."""
        with tempfile.TemporaryDirectory() as temporary:
            package = Path(temporary)
            (package / "Scripts").mkdir()
            launcher = package / "Scripts" / "tool.bat"
            launcher.write_bytes(b"@echo off\r\n")
            approved = {"Scripts/tool.bat": file_sha256(launcher)}
            self.assertEqual(verify_contents(package, set(), approved), 1)
            launcher.rename(package / "Scripts" / "other.bat")
            with self.assertRaisesRegex(ValueError, "launcher"):
                verify_contents(package, set(), approved)
            launcher.write_bytes(b"modified launcher")
            with self.assertRaisesRegex(ValueError, "launcher"):
                verify_contents(package, set(), approved)

    def test_generic_script_preserves_manpage(self) -> None:
        """Keep relocated executables and manual pages byte-identical."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            prefix = root / "prefix"
            prefix.mkdir()
            archive = root / "tool-1.0-linux-64"
            with tarfile.open(archive, "w:gz") as packed:
                for name, content, mode in [
                    ("tool", b"#!/bin/sh\nexit 0\n", 0o755),
                    ("tool.1", b".TH TOOL 1\n", 0o644),
                ]:
                    member = tarfile.TarInfo(name)
                    member.size = len(content)
                    member.mode = mode
                    packed.addfile(member, io.BytesIO(content))
            hashes = source_hashes([archive], root)
            subprocess.run(
                ["bash", str(Path(__file__).resolve().with_name("build.sh"))], cwd=root,
                env={**os.environ, "PKG_NAME": "tool", "PKG_VERSION": "1.0",
                     "target_platform": "linux-64", "PREFIX": str(prefix)},
                check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            )
            self.assertTrue((prefix / "share" / "man" / "man1" / "tool.1").is_file())
            self.assertEqual(verify_contents(prefix, hashes), 2)

    def test_upstream_checksum_mismatch_fails(self) -> None:
        """Reject an upstream download that does not match GitHub's digest."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "upstream-assets.json"
            manifest.write_text(json.dumps([{
                "url": "https://example.com/tool.zip", "sha256": "0" * 64,
            }]))
            with (
                patch(__name__ + ".urlopen", return_value=io.BytesIO(b"wrong download")),
                self.assertRaisesRegex(ValueError, "Upstream SHA-256 mismatch"),
            ):
                download_sources(manifest, root)

    def test_verification_failure_prevents_publish(self) -> None:
        """Never invoke publishing after a package fails the integrity test."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"

            def fake_build(command: list[str], *, check: bool) -> None:
                self.assertEqual(command[1], "build")
                self.assertTrue(check)
                (output / "linux-64").mkdir(parents=True)
                (output / "linux-64" / "tool.conda").touch()
                (output / "linux-64" / "tool-second.conda").touch()

            with (
                patch(__name__ + ".download_sources", return_value=[]),
                patch(__name__ + ".source_hashes", return_value={"upstream"}),
                patch(__name__ + ".verify_archive", side_effect=[None, ValueError("changed bytes")]),
                patch("subprocess.run", side_effect=fake_build) as run,
            ):
                with self.assertRaisesRegex(ValueError, "changed bytes"):
                    build_verified(root / "recipe.yaml", "linux-64", output, "https://example.com")
                self.assertEqual(run.call_count, 1)

    def test_publish_uses_verified_archive_without_rebuilding(self) -> None:
        """Publish exactly the archive that passed verification, with attestations."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            package = output / "linux-64" / "tool.conda"
            verified: list[Path] = []

            def fake_run(command: list[str], *, check: bool) -> None:
                self.assertTrue(check)
                if command[1] == "build":
                    package.parent.mkdir(parents=True)
                    package.touch()
                    (output / "src_cache").mkdir()
                    (output / "src_cache" / "download.tar.bz2").touch()
                else:
                    self.assertEqual(verified, [package])
                    self.assertEqual(command, [
                        "rattler-build", "publish", "--to", "https://example.com",
                        "--generate-attestation", str(package),
                    ])

            with (
                patch(__name__ + ".download_sources", return_value=[]),
                patch(__name__ + ".source_hashes", return_value={"upstream"}),
                patch(__name__ + ".verify_archive", side_effect=lambda package, hashes: verified.append(package)),
                patch("subprocess.run", side_effect=fake_run) as run,
            ):
                build_verified(root / "recipe.yaml", "linux-64", output, "https://example.com")
                self.assertEqual(run.call_count, 2)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    else:
        main()