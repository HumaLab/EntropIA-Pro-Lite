#!/usr/bin/env python3

from __future__ import annotations

import argparse
import csv
import io
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile
from email.parser import Parser
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[4]
TAURI_ROOT = REPO_ROOT / "apps" / "desktop" / "src-tauri"
SCRIPT_SOURCE_DIR = TAURI_ROOT / "scripts"
DEFAULT_OUTPUT_DIR = TAURI_ROOT / "target" / "runtime-payloads"
WINDOWS_PLATFORM = "windows-x86_64"
REQUIRED_SCRIPTS = (
    "paddle_vl.py",
    "spacy_ner.py",
    "transcribe.py",
)
CORE_HF_CACHE_ENTRIES = (
    # Direct model directory used by transcribe.py before falling back to HF cache.
    "Systran--faster-whisper-base",
    # HF-style cache for faster-whisper.
    "models--Systran--faster-whisper-base",
)
WINDOWS_NATIVE_LIBS = ("pdfium.dll", "onnxruntime.dll")


def default_app_data_dir() -> Path:
    appdata = os.environ.get("APPDATA")
    if appdata:
        return Path(appdata) / "com.entropia.desktop"
    return Path.home() / "AppData" / "Roaming" / "com.entropia.desktop"


def default_managed_python() -> Path:
    return default_app_data_dir() / "venv" / "entropia-env" / "Scripts" / "python.exe"


def default_uv_path() -> Path:
    return TAURI_ROOT / "resources" / "tools" / "uv" / WINDOWS_PLATFORM / "uv.exe"


def run_json(command: list[str], description: str) -> dict:
    completed = subprocess.run(
        command,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        raise ValueError(
            f"{description} failed with exit code {completed.returncode}:\n{completed.stderr.strip()}"
        )
    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise ValueError(f"{description} returned invalid JSON: {exc}\n{completed.stdout}") from exc


def inspect_managed_python(python_path: Path) -> dict:
    if not python_path.is_file():
        raise ValueError(f"managed python not found: {python_path}")

    probe = r"""
import json
import site
import sys
import sysconfig

paths = sysconfig.get_paths()
print(json.dumps({
    "executable": sys.executable,
    "prefix": sys.prefix,
    "base_prefix": sys.base_prefix,
    "version": list(sys.version_info[:3]),
    "purelib": paths.get("purelib"),
    "platlib": paths.get("platlib"),
    "sitepackages": site.getsitepackages(),
}, ensure_ascii=False))
"""
    info = run_json([str(python_path), "-c", probe], "managed python probe")
    version = info.get("version") or []
    if version[:2] != [3, 11]:
        raise ValueError(
            f"{python_path} reports Python {version}; EntropIA Windows runtime payload currently requires Python 3.11"
        )
    base_prefix = Path(info["base_prefix"])
    if not (base_prefix / "python.exe").is_file():
        raise ValueError(f"base Python runtime does not contain python.exe: {base_prefix}")
    return info


def ignore_runtime_junk(_dir: str, names: list[str]) -> set[str]:
    ignored = {"__pycache__", ".git", ".mypy_cache", ".pytest_cache"}
    ignored.update(name for name in names if name.endswith((".pyc", ".pyo")))
    return ignored


def copy_tree(source: Path, destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(source, destination, ignore=ignore_runtime_junk)


def copy_python_runtime(info: dict, payload_root: Path) -> None:
    source = Path(info["base_prefix"])
    destination = payload_root / "python"
    copy_tree(source, destination)
    if not (destination / "python.exe").is_file():
        raise ValueError(f"copied Python runtime is missing python.exe: {destination}")


def copy_uv(uv_path: Path, payload_root: Path) -> None:
    if not uv_path.is_file():
        raise ValueError(f"uv binary not found: {uv_path}")
    destination = payload_root / "uv"
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(uv_path, destination / "uv.exe")


def ensure_repo_scripts(payload_root: Path) -> None:
    scripts_dir = payload_root / "scripts"
    scripts_dir.mkdir(parents=True, exist_ok=True)
    for script_name in REQUIRED_SCRIPTS:
        source = SCRIPT_SOURCE_DIR / script_name
        if not source.is_file():
            raise ValueError(f"required repo script missing: {source}")
        shutil.copy2(source, scripts_dir / script_name)


def parse_email_file(path: Path):
    return Parser().parsestr(path.read_text(encoding="utf-8", errors="replace"))


def wheel_safe(value: str) -> str:
    return re.sub(r"[^\w\d.]+", "_", value, flags=re.UNICODE).strip("_")


def tag_safe(value: str) -> str:
    return re.sub(r"[^\w\d.-]+", "_", value, flags=re.UNICODE).strip("_")


def dist_name_and_version(dist_info: Path) -> tuple[str, str]:
    metadata_path = dist_info / "METADATA"
    if not metadata_path.is_file():
        raise ValueError(f"missing METADATA in {dist_info}")
    metadata = parse_email_file(metadata_path)
    name = metadata.get("Name")
    version = metadata.get("Version")
    if not name or not version:
        raise ValueError(f"METADATA missing Name/Version in {dist_info}")
    return wheel_safe(name), wheel_safe(version)


def wheel_tags(dist_info: Path) -> str:
    wheel_path = dist_info / "WHEEL"
    if not wheel_path.is_file():
        return "py3-none-any"
    wheel = parse_email_file(wheel_path)
    tags = wheel.get_all("Tag") or ["py3-none-any"]
    parsed = []
    for tag in tags:
        parts = tag.split("-")
        if len(parts) != 3:
            return tag_safe(tags[0])
        parsed.append(tuple(parts))
    python_tags = ".".join(dict.fromkeys(tag[0] for tag in parsed))
    abi_tags = ".".join(dict.fromkeys(tag[1] for tag in parsed))
    platform_tags = ".".join(dict.fromkeys(tag[2] for tag in parsed))
    return tag_safe(f"{python_tags}-{abi_tags}-{platform_tags}")


def record_paths(site_packages: Path, dist_info: Path) -> list[Path]:
    record_path = dist_info / "RECORD"
    if not record_path.is_file():
        return sorted(path for path in dist_info.rglob("*") if path.is_file())

    paths: list[Path] = []
    with record_path.open("r", encoding="utf-8", newline="") as handle:
        for row in csv.reader(handle):
            if not row:
                continue
            rel = row[0].replace("/", os.sep)
            candidate = site_packages / rel
            if candidate.is_file():
                paths.append(candidate)
    return sorted(set(paths))


def repack_site_packages_as_wheels(site_packages: Path, wheelhouse: Path) -> list[str]:
    if not site_packages.is_dir():
        raise ValueError(f"site-packages not found: {site_packages}")
    if wheelhouse.exists():
        shutil.rmtree(wheelhouse)
    wheelhouse.mkdir(parents=True, exist_ok=True)

    created: list[str] = []
    for dist_info in sorted(site_packages.glob("*.dist-info")):
        name, version = dist_name_and_version(dist_info)
        tags = wheel_tags(dist_info)
        wheel_name = f"{name}-{version}-{tags}.whl"
        wheel_path = wheelhouse / wheel_name
        files = record_paths(site_packages, dist_info)
        if not files:
            continue
        with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for file_path in files:
                archive.write(file_path, file_path.relative_to(site_packages).as_posix())
        created.append(wheel_name)

    if not created:
        raise ValueError(f"no distributions were repacked from {site_packages}")
    return created


def select_site_packages(info: dict) -> Path:
    candidates = []
    for key in ("purelib", "platlib"):
        value = info.get(key)
        if value:
            candidates.append(Path(value))
    for value in info.get("sitepackages") or []:
        candidates.append(Path(value))
    for candidate in candidates:
        if candidate.is_dir() and any(candidate.glob("*.dist-info")):
            return candidate
    raise ValueError("managed Python environment has no discoverable site-packages with dist-info metadata")


def copy_cache_dir(source: Path | None, destination: Path, selected_entries: tuple[str, ...] | None = None) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True, exist_ok=True)

    if source is None or not source.is_dir():
        (destination / "CACHE_NOT_SEEDED.txt").write_text(
            "Cache source was not available when materializing this runtime payload.\n",
            encoding="utf-8",
        )
        return

    copied = False
    if selected_entries is None:
        for child in sorted(source.iterdir()):
            target = destination / child.name
            if child.is_dir():
                shutil.copytree(child, target, ignore=ignore_runtime_junk)
            elif child.is_file():
                shutil.copy2(child, target)
            copied = True
    else:
        for entry in selected_entries:
            child = source / entry
            if not child.exists():
                continue
            target = destination / child.name
            if child.is_dir():
                shutil.copytree(child, target, ignore=ignore_runtime_junk)
            elif child.is_file():
                shutil.copy2(child, target)
            copied = True

    if not copied:
        (destination / "CACHE_NOT_SEEDED.txt").write_text(
            f"No selected cache entries were found in {source}.\n",
            encoding="utf-8",
        )


def copy_caches(
    payload_root: Path,
    app_data_dir: Path,
    hf_cache_mode: str,
    hf_cache_source: Path | None = None,
    paddlex_cache_source: Path | None = None,
) -> None:
    caches_root = payload_root / "caches"
    hf_source = hf_cache_source or app_data_dir / "hf_cache"
    paddlex_source = paddlex_cache_source or app_data_dir / "paddlex_cache"

    selected_hf = None if hf_cache_mode == "all" else CORE_HF_CACHE_ENTRIES
    if hf_cache_mode == "none":
        selected_hf = ()

    copy_cache_dir(hf_source, caches_root / "hf", selected_hf)
    copy_cache_dir(paddlex_source, caches_root / "paddlex", None)


def copy_native_libs(payload_root: Path, native_source_dirs: list[Path]) -> None:
    destination = payload_root / "resources" / "lib"
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True, exist_ok=True)

    missing = []
    for lib_name in WINDOWS_NATIVE_LIBS:
        found = None
        for source_dir in native_source_dirs:
            candidate = source_dir / lib_name
            if candidate.is_file():
                found = candidate
                break
        if found is None:
            missing.append(lib_name)
            continue
        shutil.copy2(found, destination / lib_name)

    if missing:
        raise ValueError(
            "missing Windows native runtime libraries: "
            + ", ".join(missing)
            + ". Provide --native-source-dir paths containing them."
        )


def write_manifest_overrides(payload_root: Path, pack_version: str, app_version: str) -> None:
    overrides = {
        "pack_version": pack_version,
        "app_version": app_version,
        "payload_profile": "release",
        "release_injection_required": False,
        "external_artifacts_required": [],
        "python_relpath": "python/python.exe",
        "uv_relpath": "uv/uv.exe",
    }
    (payload_root / "manifest.overrides.json").write_text(
        json.dumps(overrides, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def materialize_payload(
    output_dir: Path,
    managed_python: Path,
    uv_path: Path,
    pack_version: str,
    app_version: str,
    app_data_dir: Path,
    native_source_dirs: list[Path],
    hf_cache_mode: str,
    hf_cache_source: Path | None = None,
    paddlex_cache_source: Path | None = None,
) -> Path:
    payload_root = output_dir / WINDOWS_PLATFORM
    if payload_root.exists():
        shutil.rmtree(payload_root)
    payload_root.mkdir(parents=True)

    info = inspect_managed_python(managed_python)
    copy_python_runtime(info, payload_root)
    copy_uv(uv_path, payload_root)
    ensure_repo_scripts(payload_root)
    repack_site_packages_as_wheels(select_site_packages(info), payload_root / "wheelhouse")
    copy_caches(
        payload_root,
        app_data_dir,
        hf_cache_mode,
        hf_cache_source=hf_cache_source,
        paddlex_cache_source=paddlex_cache_source,
    )
    copy_native_libs(payload_root, native_source_dirs)
    write_manifest_overrides(payload_root, pack_version, app_version)
    return payload_root


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Materialize a Windows x86_64 EntropIA release runtime payload from the "
            "currently working managed venv. The output is intended for "
            "build_runtime_pack.py --payload-root, not for committing to git."
        )
    )
    parser.add_argument("--output-dir", default=str(DEFAULT_OUTPUT_DIR))
    parser.add_argument("--managed-python", default=str(default_managed_python()))
    parser.add_argument("--uv-path", default=str(default_uv_path()))
    parser.add_argument("--pack-version", required=True)
    parser.add_argument("--app-version", required=True)
    parser.add_argument("--app-data-dir", default=str(default_app_data_dir()))
    parser.add_argument("--hf-cache-source")
    parser.add_argument("--paddlex-cache-source")
    parser.add_argument(
        "--native-source-dir",
        action="append",
        default=[
            str(TAURI_ROOT / "resources" / "lib"),
            str(TAURI_ROOT / "resources" / "models" / "ner"),
        ],
        help="Directory to search for pdfium.dll/onnxruntime.dll. Can be repeated.",
    )
    parser.add_argument(
        "--hf-cache-mode",
        choices=("core", "all", "none"),
        default="core",
        help="Copy core HF cache entries by default; use all only when size is acceptable.",
    )
    args = parser.parse_args()

    try:
        payload_root = materialize_payload(
            output_dir=Path(args.output_dir),
            managed_python=Path(args.managed_python),
            uv_path=Path(args.uv_path),
            pack_version=args.pack_version,
            app_version=args.app_version,
            app_data_dir=Path(args.app_data_dir),
            native_source_dirs=[Path(path) for path in args.native_source_dir],
            hf_cache_mode=args.hf_cache_mode,
            hf_cache_source=Path(args.hf_cache_source) if args.hf_cache_source else None,
            paddlex_cache_source=Path(args.paddlex_cache_source) if args.paddlex_cache_source else None,
        )
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    print(json.dumps({"materialized": str(payload_root)}, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
