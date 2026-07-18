#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[4]
TAURI_ROOT = REPO_ROOT / 'apps' / 'desktop' / 'src-tauri'
SCRIPT_SOURCE_DIR = TAURI_ROOT / 'scripts'
SUPPORTED_PLATFORMS = ('windows-x86_64', 'linux-x86_64')
REQUIRED_SCRIPTS = ('paddle_vl.py', 'spacy_ner.py', 'transcribe.py')
REQUIRED_WHEEL_PREFIXES = (
    'es_core_news_md',
    'paddleocr',
    'paddlepaddle',
    'faster_whisper',
    'spacy',
)
REQUIRED_CACHE_DIRS = ('hf', 'paddlex')
REQUIRED_LAYOUT_DIRS = ('python', 'uv', 'scripts', 'wheelhouse', 'caches', 'resources/lib')
OVERRIDES_FILENAME = 'manifest.overrides.json'
SPACY_MODEL_WHEEL_URL = 'https://github.com/explosion/spacy-models/releases/download/es_core_news_md-3.8.0/es_core_news_md-3.8.0-py3-none-any.whl'
SPACY_RUNTIME_SPEC = 'spacy>=3.8.0,<3.9.0'


def platform_python_relpath(platform: str) -> str:
    return 'python/python.exe' if platform == 'windows-x86_64' else 'python/bin/python3'


def platform_uv_relpath(platform: str) -> str:
    return 'uv/uv.exe' if platform == 'windows-x86_64' else 'uv/bin/uv'


def platform_native_lib_names(platform: str) -> tuple[str, ...]:
    if platform == 'windows-x86_64':
        return ('pdfium.dll', 'onnxruntime.dll')
    return ('libpdfium.so', 'libonnxruntime.so')


def normalized_name(path: Path) -> str:
    return path.name.lower().replace('-', '_')


def is_executable_for_platform(path: Path, platform: str) -> bool:
    if platform == 'windows-x86_64':
        return path.suffix.lower() in {'.exe', '.bat', '.cmd', '.ps1'}
    return os.access(path, os.X_OK)


def make_executable(path: Path) -> None:
    path.chmod(path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def resolve_source_root(payload_source_dir: Path, platform: str) -> Path:
    platform_root = payload_source_dir / platform
    if platform_root.exists():
        return platform_root
    if any((payload_source_dir / rel).exists() for rel in REQUIRED_LAYOUT_DIRS):
        return payload_source_dir
    raise ValueError(
        f'{platform}: payload source {payload_source_dir} does not contain a {platform}/ directory or direct payload layout'
    )


def copy_tree_contents(source: Path, destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    for child in sorted(source.iterdir()):
        target = destination / child.name
        if child.is_dir():
            shutil.copytree(child, target, ignore=shutil.ignore_patterns('__pycache__', '*.pyc', '*.pyo'))
        else:
            if child.suffix.lower() in {'.pyc', '.pyo'}:
                continue
            shutil.copy2(child, target)


def ensure_repo_scripts(payload_root: Path) -> None:
    scripts_dir = payload_root / 'scripts'
    scripts_dir.mkdir(parents=True, exist_ok=True)
    for script_name in REQUIRED_SCRIPTS:
        source = SCRIPT_SOURCE_DIR / script_name
        if not source.exists():
            raise ValueError(f'required repo script missing: {source}')
        shutil.copy2(source, scripts_dir / script_name)


def ensure_spacy_wheelhouse(platform: str, payload_root: Path) -> None:
    wheelhouse = payload_root / 'wheelhouse'
    wheelhouse.mkdir(parents=True, exist_ok=True)
    wheel_names = [normalized_name(path) for path in wheelhouse.iterdir() if path.is_file()]
    if any(name.startswith('spacy_3.8') for name in wheel_names) and any(
        name.startswith('es_core_news_md') for name in wheel_names
    ):
        return

    cmd = [
        sys.executable,
        '-m',
        'pip',
        'download',
        '--only-binary=:all:',
        '--dest',
        str(wheelhouse),
    ]
    if platform == 'windows-x86_64':
        cmd.extend([
            '--platform',
            'win_amd64',
            '--python-version',
            '311',
            '--implementation',
            'cp',
            '--abi',
            'cp311',
        ])
    cmd.extend([SPACY_RUNTIME_SPEC, SPACY_MODEL_WHEEL_URL])

    subprocess.run(cmd, check=True)


def validate_required_payload(platform: str, payload_root: Path) -> None:
    errors: list[str] = []

    for rel_dir in REQUIRED_LAYOUT_DIRS:
        if not (payload_root / rel_dir).is_dir():
            errors.append(f'missing directory: {rel_dir}')

    python_path = payload_root / platform_python_relpath(platform)
    uv_path = payload_root / platform_uv_relpath(platform)
    if not python_path.is_file():
        errors.append(f'missing python executable: {platform_python_relpath(platform)}')
    elif not is_executable_for_platform(python_path, platform):
        errors.append(f'python executable is not executable for {platform}: {platform_python_relpath(platform)}')
    if not uv_path.is_file():
        errors.append(f'missing uv executable: {platform_uv_relpath(platform)}')
    elif not is_executable_for_platform(uv_path, platform):
        errors.append(f'uv executable is not executable for {platform}: {platform_uv_relpath(platform)}')

    for script_name in REQUIRED_SCRIPTS:
        if not (payload_root / 'scripts' / script_name).is_file():
            errors.append(f'missing script: scripts/{script_name}')

    wheelhouse = payload_root / 'wheelhouse'
    wheel_names = [normalized_name(path) for path in wheelhouse.iterdir() if path.is_file()] if wheelhouse.is_dir() else []
    for prefix in REQUIRED_WHEEL_PREFIXES:
        if not any(name.startswith(prefix) for name in wheel_names):
            errors.append(f'missing wheelhouse artifact matching: {prefix}*')

    for cache_dir in REQUIRED_CACHE_DIRS:
        cache_root = payload_root / 'caches' / cache_dir
        if not cache_root.is_dir():
            errors.append(f'missing cache directory: caches/{cache_dir}')
        elif not any(path.is_file() for path in cache_root.rglob('*')):
            errors.append(f'cache directory has no files: caches/{cache_dir}')

    native_dir = payload_root / 'resources' / 'lib'
    native_names = {path.name.lower() for path in native_dir.iterdir() if path.is_file()} if native_dir.is_dir() else set()
    for lib_name in platform_native_lib_names(platform):
        if lib_name.lower() not in native_names:
            errors.append(f'missing native library: resources/lib/{lib_name}')

    if errors:
        joined = '\n  - '.join(errors)
        raise ValueError(f'{platform}: runtime payload source is incomplete:\n  - {joined}')


def write_manifest_overrides(platform: str, payload_root: Path, pack_version: str, app_version: str) -> None:
    overrides = {
        'pack_version': pack_version,
        'app_version': app_version,
        'payload_profile': 'release',
        'release_injection_required': False,
        'external_artifacts_required': [],
        'python_relpath': platform_python_relpath(platform),
        'uv_relpath': platform_uv_relpath(platform),
    }
    (payload_root / OVERRIDES_FILENAME).write_text(
        json.dumps(overrides, indent=2, ensure_ascii=False) + '\n',
        encoding='utf-8',
    )


def write_text(path: Path, content: str, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding='utf-8')
    if executable:
        make_executable(path)


def create_fixture_payload(platform: str, payload_root: Path) -> None:
    if payload_root.exists():
        shutil.rmtree(payload_root)
    payload_root.mkdir(parents=True)

    if platform == 'windows-x86_64':
        write_text(payload_root / platform_python_relpath(platform), '@echo off\necho Python 3.12.0\n', executable=True)
        write_text(payload_root / platform_uv_relpath(platform), '@echo off\necho uv 0.6.14\n', executable=True)
    else:
        write_text(payload_root / platform_python_relpath(platform), '#!/bin/sh\necho Python 3.12.0\n', executable=True)
        write_text(payload_root / platform_uv_relpath(platform), '#!/bin/sh\necho uv 0.6.14\n', executable=True)

    ensure_repo_scripts(payload_root)

    for prefix in REQUIRED_WHEEL_PREFIXES:
        write_text(payload_root / 'wheelhouse' / f'{prefix}-0.0.0-py3-none-any.whl', f'fixture wheel for {prefix}\n')
    for cache_dir in REQUIRED_CACHE_DIRS:
        write_text(payload_root / 'caches' / cache_dir / 'fixture-cache.txt', f'fixture cache for {cache_dir}\n')
    for lib_name in platform_native_lib_names(platform):
        write_text(payload_root / 'resources' / 'lib' / lib_name, f'fixture native library for {lib_name}\n')
    write_text(
        payload_root / 'PAYLOAD_FIXTURE_ONLY.txt',
        'TEST-ONLY fixture payload generated by prepare_runtime_payload.py --fixture. Do not ship in releases.\n',
    )


def prepare_payload(
    platform: str,
    output_dir: Path,
    pack_version: str,
    app_version: str,
    payload_source_dir: Path | None = None,
    fixture: bool = False,
) -> Path:
    destination = output_dir / platform
    if fixture:
        create_fixture_payload(platform, destination)
    else:
        if payload_source_dir is None:
            raise ValueError(f'{platform}: --payload-source-dir is required unless --fixture is used')
        source = resolve_source_root(payload_source_dir, platform)
        copy_tree_contents(source, destination)
        ensure_repo_scripts(destination)
        ensure_spacy_wheelhouse(platform, destination)

    validate_required_payload(platform, destination)
    write_manifest_overrides(platform, destination, pack_version, app_version)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description='Prepare repo-owned EntropIA runtime payloads for release pack assembly.')
    parser.add_argument('--platform', required=True, choices=SUPPORTED_PLATFORMS)
    parser.add_argument('--output-dir', required=True)
    parser.add_argument('--pack-version', required=True)
    parser.add_argument('--app-version', required=True)
    parser.add_argument('--payload-source-dir')
    parser.add_argument('--fixture', action='store_true', help='Generate tiny test-only payload files for CI/script tests.')
    args = parser.parse_args()

    try:
        output = prepare_payload(
            platform=args.platform,
            output_dir=Path(args.output_dir),
            pack_version=args.pack_version,
            app_version=args.app_version,
            payload_source_dir=Path(args.payload_source_dir) if args.payload_source_dir else None,
            fixture=args.fixture,
        )
    except ValueError as exc:
        print(f'error: {exc}', file=sys.stderr)
        return 1
    print(json.dumps({'prepared': str(output)}, indent=2, ensure_ascii=False))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
