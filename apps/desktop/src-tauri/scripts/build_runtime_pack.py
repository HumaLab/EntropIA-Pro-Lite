#!/usr/bin/env python3

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
from pathlib import Path
from typing import Iterable


REPO_ROOT = Path(__file__).resolve().parents[4]
TAURI_ROOT = REPO_ROOT / 'apps' / 'desktop' / 'src-tauri'
RUNTIME_PACK_ROOT = TAURI_ROOT / 'resources' / 'runtime-pack'
DIST_ROOT = TAURI_ROOT / 'target' / 'runtime-pack'
SUPPORTED_PLATFORMS = ('windows-x86_64', 'linux-x86_64')
SCRIPT_SOURCE_DIR = TAURI_ROOT / 'scripts'
REQUIRED_SCRIPTS = ('paddle_vl.py', 'spacy_ner.py', 'transcribe.py')
OVERRIDES_FILENAME = 'manifest.overrides.json'
CATEGORY_DIRS = {
    'python_files': ('python',),
    'uv_files': ('uv',),
    'script_files': ('scripts',),
    'wheelhouse': ('wheelhouse',),
    'caches': ('caches',),
    'native_assets': ('resources/lib', 'resources/models/ocr'),
}
REPO_RUNTIME_RESOURCE_DIRS = (
    ('resources/models/ocr', TAURI_ROOT / 'resources' / 'models' / 'ocr'),
)


def is_generated_python_cache(path: Path) -> bool:
    parts = set(path.parts)
    return '__pycache__' in parts or path.suffix.lower() in {'.pyc', '.pyo'}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as handle:
        for chunk in iter(lambda: handle.read(65536), b''):
            digest.update(chunk)
    return digest.hexdigest()


def iter_manifest_entries(manifest: dict) -> Iterable[dict]:
    for key in CATEGORY_DIRS:
        yield from manifest.get(key, [])


def infer_executable(path: Path) -> bool:
    if os.name == 'nt':
        return path.suffix.lower() in {'.exe', '.bat', '.cmd', '.ps1'}
    return os.access(path, os.X_OK)


def collect_entries(root: Path, relative_dirs: Iterable[str]) -> list[dict]:
    entries = []
    for relative_dir in relative_dirs:
        base = root / relative_dir
        if not base.exists():
            continue

        for path in sorted(candidate for candidate in base.rglob('*') if candidate.is_file()):
            if relative_dir == 'python' and is_generated_python_cache(path.relative_to(root)):
                continue
            entries.append(
                {
                    'path': path.relative_to(root).as_posix(),
                    'sha256': sha256_file(path),
                    'size': path.stat().st_size,
                    'executable': infer_executable(path),
                }
            )
    return entries


def validate_manifest_contract(platform: str, manifest: dict, require_release: bool = False) -> None:
    if manifest.get('platform') != platform:
        raise ValueError(f'{platform}: manifest platform mismatch ({manifest.get("platform")})')

    python_relpath = manifest.get('python_relpath')
    uv_relpath = manifest.get('uv_relpath')
    python_paths = {entry['path'] for entry in manifest.get('python_files', [])}
    uv_paths = {entry['path'] for entry in manifest.get('uv_files', [])}

    if not python_relpath:
        raise ValueError(f'{platform}: manifest missing python_relpath')
    if not uv_relpath:
        raise ValueError(f'{platform}: manifest missing uv_relpath')
    if python_relpath not in python_paths:
        raise ValueError(f'{platform}: python_relpath missing from python_files ({python_relpath})')
    if uv_relpath not in uv_paths:
        raise ValueError(f'{platform}: uv_relpath missing from uv_files ({uv_relpath})')

    if require_release or manifest.get('payload_profile') == 'release':
        if manifest.get('payload_profile') != 'release':
            raise ValueError(f'{platform}: release runtime-pack must set payload_profile=release')
        if manifest.get('release_injection_required') is not False:
            raise ValueError(f'{platform}: release runtime-pack must set release_injection_required=false')
        if manifest.get('external_artifacts_required') != []:
            raise ValueError(
                f'{platform}: release runtime-pack cannot keep external_artifacts_required once injection is complete'
            )
        for key in ('python_files', 'uv_files', 'wheelhouse', 'script_files'):
            if not manifest.get(key):
                raise ValueError(f'{platform}: release runtime-pack must include non-empty {key}')


def load_manifest(platform: str, root: Path, require_release: bool = False) -> dict:
    manifest_path = root / 'manifest.json'
    manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
    validate_manifest_contract(platform, manifest, require_release=require_release)
    return manifest


def validate_manifest(platform: str, root: Path, require_release: bool = False) -> dict:
    manifest = load_manifest(platform, root, require_release=require_release)

    missing = []
    mismatched = []
    for entry in iter_manifest_entries(manifest):
        target = root / entry['path']
        if not target.exists():
            missing.append(entry['path'])
            continue
        if target.stat().st_size != entry['size']:
            mismatched.append(f"size:{entry['path']}")
        if sha256_file(target) != entry['sha256']:
            mismatched.append(f"sha256:{entry['path']}")

    if missing or mismatched:
        detail = []
        if missing:
            detail.append(f'missing={missing}')
        if mismatched:
            detail.append(f'mismatched={mismatched}')
        raise ValueError(f'{platform}: invalid fixture runtime-pack ({"; ".join(detail)})')

    return manifest


def resolve_payload_root(payload_root: Path, platform: str) -> Path:
    platform_root = payload_root / platform
    if platform_root.exists():
        return platform_root

    if any((payload_root / rel_dir).exists() for rel_dirs in CATEGORY_DIRS.values() for rel_dir in rel_dirs) or (
        payload_root / OVERRIDES_FILENAME
    ).exists():
        return payload_root

    raise ValueError(
        f'{platform}: payload root {payload_root} does not contain a {platform}/ directory or a direct runtime-pack layout'
    )


def overlay_payload(payload_root: Path, destination: Path) -> None:
    for source in sorted(payload_root.rglob('*')):
        if source.is_dir():
            continue
        relative = source.relative_to(payload_root)
        if relative.as_posix() in {'manifest.json', OVERRIDES_FILENAME}:
            continue

        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)


def overlay_repo_runtime_resources(destination: Path) -> None:
    for relative_dir, source_dir in REPO_RUNTIME_RESOURCE_DIRS:
        if not source_dir.exists():
            continue

        target_dir = destination / relative_dir
        if target_dir.exists():
            shutil.rmtree(target_dir)
        target_dir.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(source_dir, target_dir)


def overlay_repo_scripts(destination: Path) -> None:
    scripts_dir = destination / 'scripts'
    scripts_dir.mkdir(parents=True, exist_ok=True)
    for script_name in REQUIRED_SCRIPTS:
        source = SCRIPT_SOURCE_DIR / script_name
        if not source.is_file():
            raise ValueError(f'required repo script missing: {source}')
        shutil.copy2(source, scripts_dir / script_name)


def load_manifest_overrides(payload_root: Path | None) -> dict:
    if payload_root is None:
        return {}

    overrides_path = payload_root / OVERRIDES_FILENAME
    if not overrides_path.exists():
        return {}

    overrides = json.loads(overrides_path.read_text(encoding='utf-8'))
    allowed_keys = {
        'pack_version',
        'app_version',
        'payload_profile',
        'release_injection_required',
        'external_artifacts_required',
        'python_relpath',
        'uv_relpath',
    }
    unexpected = sorted(set(overrides) - allowed_keys)
    if unexpected:
        raise ValueError(
            f'Unsupported manifest override keys in {overrides_path}: {", ".join(unexpected)}'
        )
    return overrides


def load_release_manifest_overrides(platform: str, payload_root: Path | None) -> dict:
    if payload_root is None:
        raise ValueError(f'{platform}: --require-release-payload requires --payload-root')

    overrides_path = payload_root / OVERRIDES_FILENAME
    if not overrides_path.exists():
        raise ValueError(
            f'{platform}: release payload {payload_root} must include {OVERRIDES_FILENAME}'
        )

    return load_manifest_overrides(payload_root)


def regenerate_manifest(base_manifest: dict, destination: Path, overrides: dict, require_release: bool = False) -> dict:
    manifest = dict(base_manifest)
    manifest.update(overrides)

    for key, rel_dirs in CATEGORY_DIRS.items():
        manifest[key] = collect_entries(destination, rel_dirs)

    validate_manifest_contract(manifest['platform'], manifest, require_release=require_release)
    return manifest


def build_platform(
    platform: str,
    output_root: Path,
    payload_root: Path | None,
    fixture_root: Path = RUNTIME_PACK_ROOT,
    require_release_payload: bool = False,
) -> Path:
    resolved_payload_root = None
    overrides = {}
    if payload_root is not None:
        resolved_payload_root = resolve_payload_root(payload_root, platform)

    if require_release_payload:
        overrides = load_release_manifest_overrides(platform, resolved_payload_root)
    elif resolved_payload_root is not None:
        overrides = load_manifest_overrides(resolved_payload_root)

    source = fixture_root / platform
    fixture_manifest = load_manifest(platform, source)

    destination = output_root / platform
    if destination.resolve() == source.resolve():
        raise ValueError(
            f'{platform}: output directory {destination} must differ from source fixture directory {source}'
        )
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(source, destination)

    if resolved_payload_root is not None:
        overlay_payload(resolved_payload_root, destination)

    overlay_repo_runtime_resources(destination)
    overlay_repo_scripts(destination)

    manifest = regenerate_manifest(
        fixture_manifest,
        destination,
        overrides,
        require_release=require_release_payload,
    )
    (destination / 'manifest.json').write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + '\n',
        encoding='utf-8',
    )
    validate_manifest(platform, destination, require_release=require_release_payload)

    summary = {
        'platform': platform,
        'pack_version': manifest['pack_version'],
        'app_version': manifest.get('app_version'),
        'payload_profile': manifest.get('payload_profile', 'release'),
        'release_injection_required': manifest.get('release_injection_required', False),
        'external_artifacts_required': manifest.get('external_artifacts_required', []),
        'payload_root': str(resolved_payload_root) if resolved_payload_root else None,
        'files': [entry['path'] for entry in iter_manifest_entries(manifest)],
    }
    (destination / 'assembly-summary.json').write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + '\n',
        encoding='utf-8',
    )
    return destination


def refresh_fixture_scripts(platform: str, fixture_root: Path = RUNTIME_PACK_ROOT) -> Path:
    destination = fixture_root / platform
    fixture_manifest = load_manifest(platform, destination)

    overlay_repo_scripts(destination)
    manifest = regenerate_manifest(fixture_manifest, destination, {})
    (destination / 'manifest.json').write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + '\n',
        encoding='utf-8',
    )
    validate_manifest(platform, destination)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description='Assemble EntropIA runtime-pack fixtures for CI/release wiring.')
    parser.add_argument('--platform', action='append', choices=SUPPORTED_PLATFORMS, dest='platforms')
    parser.add_argument('--output-dir', default=str(DIST_ROOT))
    parser.add_argument(
        '--fixture-root',
        default=str(RUNTIME_PACK_ROOT),
        help='Fixture runtime-pack parent directory. Defaults to bundled resources/runtime-pack.',
    )
    parser.add_argument(
        '--payload-root',
        help='Optional external payload directory. Accepts either <root>/<platform>/... or a direct platform payload layout plus manifest.overrides.json.',
    )
    parser.add_argument(
        '--require-release-payload',
        action='store_true',
        help='Require a real release payload with manifest overrides and fail fixture-only packs.',
    )
    parser.add_argument(
        '--refresh-fixture-scripts',
        action='store_true',
        help='Refresh only fixture runtime-pack scripts and manifests from canonical repository sources.',
    )
    args = parser.parse_args()

    output_root = Path(args.output_dir)
    output_root.mkdir(parents=True, exist_ok=True)
    fixture_root = Path(args.fixture_root)
    platforms = args.platforms or list(SUPPORTED_PLATFORMS)
    payload_root = Path(args.payload_root) if args.payload_root else None

    try:
        if args.refresh_fixture_scripts:
            if payload_root is not None or args.require_release_payload:
                raise ValueError('--refresh-fixture-scripts cannot be combined with release payload options')
            assembled = [
                str(refresh_fixture_scripts(platform, fixture_root=fixture_root))
                for platform in platforms
            ]
        else:
            assembled = [
                str(
                    build_platform(
                        platform,
                        output_root,
                        payload_root,
                        fixture_root=fixture_root,
                        require_release_payload=args.require_release_payload,
                    )
                )
                for platform in platforms
            ]
    except ValueError as exc:
        print(f'error: {exc}', file=sys.stderr)
        return 1

    print(json.dumps({'assembled': assembled}, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
