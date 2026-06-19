#!/usr/bin/env python3

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform as host_platform
import subprocess
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[4]
TAURI_ROOT = REPO_ROOT / 'apps' / 'desktop' / 'src-tauri'
RUNTIME_PACK_ROOT = TAURI_ROOT / 'resources' / 'runtime-pack'
REQUIRED_RELEASE_SCRIPTS = {
    'scripts/paddle_vl.py',
    'scripts/spacy_ner.py',
    'scripts/transcribe.py',
}
REQUIRED_RELEASE_WHEELS = {
    'es_core_news_md',
    'paddleocr',
    'paddlepaddle',
    'faster_whisper',
    'spacy',
}
REQUIRED_RELEASE_CACHE_DIRS = (
    'caches/hf',
    'caches/paddlex',
)
REQUIRED_RELEASE_NATIVE_ASSETS = {
    'resources/models/ocr/PP-OCRv5_mobile_det.mnn',
    'resources/models/ocr/latin_PP-OCRv5_mobile_rec_infer.mnn',
    'resources/models/ocr/ppocr_keys_latin.txt',
    'resources/models/ocr/PP-LCNet_x1_0_doc_ori.mnn',
}
CACHE_NOT_SEEDED_MARKER = 'CACHE_NOT_SEEDED.txt'
INSTALL_PROBE_SPECS = (
    'paddlepaddle>=3.2.1,<3.3.0',
    'paddleocr[doc-parser]>=2.9.0',
    'faster-whisper>=1.0.0',
    # spaCy must be installed explicitly: the es_core_news_md 3.8 model wheel no
    # longer declares spaCy as a dependency (mirrors src/deps/registry.rs).
    'spacy>=3.8.0,<3.9.0',
    'https://github.com/explosion/spacy-models/releases/download/es_core_news_md-3.8.0/es_core_news_md-3.8.0-py3-none-any.whl',
)
INSTALL_PROBE_IMPORTS = (
    'import paddle; from paddleocr import PaddleOCRVL; print("paddleocr ok")',
    'import faster_whisper, ctranslate2; print("faster_whisper ok")',
    'import spacy; spacy.load("es_core_news_md"); print("spacy ok")',
)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open('rb') as handle:
        for chunk in iter(lambda: handle.read(65536), b''):
            digest.update(chunk)
    return digest.hexdigest()


def iter_manifest_entries(manifest: dict) -> list[dict]:
    entries = []
    for key in ('python_files', 'uv_files', 'script_files', 'wheelhouse', 'caches', 'native_assets'):
        entries.extend(manifest.get(key, []))
    return entries


def resolve_pack_root(root: Path, platform: str) -> Path:
    direct = root / 'manifest.json'
    if root.name == platform and direct.exists():
        return root
    return root / platform


def load_manifest(root: Path, platform: str) -> dict:
    manifest_path = resolve_pack_root(root, platform) / 'manifest.json'
    return json.loads(manifest_path.read_text(encoding='utf-8'))


def required_paths(manifest: dict) -> list[str]:
    return [
        manifest['python_relpath'],
        manifest['uv_relpath'],
        'scripts/paddle_vl.py',
        'scripts/spacy_ner.py',
        'scripts/transcribe.py',
    ]


def current_host_pack_platform() -> str | None:
    system = host_platform.system().lower()
    machine = host_platform.machine().lower()
    if machine not in {'x86_64', 'amd64'}:
        return None
    if system == 'linux':
        return 'linux-x86_64'
    if system == 'windows':
        return 'windows-x86_64'
    return None


def run_version_probe(executable: Path, expected_platform: str) -> dict:
    probe = {
        'path': str(executable),
        'host_compatible': current_host_pack_platform() == expected_platform,
        'attempted': False,
        'ok': None,
        'stdout': '',
        'stderr': '',
        'error': None,
    }
    if not probe['host_compatible']:
        return probe

    probe['attempted'] = True
    try:
        completed = subprocess.run(
            [str(executable), '--version'],
            check=False,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=20,
        )
    except Exception as exc:  # noqa: BLE001 - diagnostics should preserve any probe failure
        probe['ok'] = False
        probe['error'] = str(exc)
        return probe

    probe['ok'] = completed.returncode == 0
    probe['stdout'] = completed.stdout.strip()
    probe['stderr'] = completed.stderr.strip()
    return probe


def run_command(args: list[str], env: dict[str, str]) -> dict:
    completed = subprocess.run(
        args,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=600,
        env=env,
    )
    return {
        'args': args,
        'ok': completed.returncode == 0,
        'returncode': completed.returncode,
        'stdout': completed.stdout.strip(),
        'stderr': completed.stderr.strip(),
    }


def run_install_probe(pack_root: Path, manifest: dict, expected_platform: str) -> dict:
    probe = {
        'host_compatible': current_host_pack_platform() == expected_platform,
        'attempted': False,
        'ok': None,
        'steps': [],
        'error': None,
    }
    if not probe['host_compatible']:
        return probe

    python_path = pack_root / manifest['python_relpath']
    uv_path = pack_root / manifest['uv_relpath']
    wheelhouse = pack_root / 'wheelhouse'

    probe['attempted'] = True
    with tempfile.TemporaryDirectory(prefix='entropia-runtime-probe-') as temp_dir:
        temp_root = Path(temp_dir)
        venv_dir = temp_root / 'venv'
        venv_python = venv_dir / ('Scripts/python.exe' if expected_platform == 'windows-x86_64' else 'bin/python')
        env = dict(os.environ)
        env['UV_CACHE_DIR'] = str(temp_root / 'uv-cache')

        commands = [
            [str(uv_path), 'venv', str(venv_dir), '--python', str(python_path), '--offline'],
        ]
        commands.extend(
            [
                str(uv_path),
                'pip',
                'install',
                spec,
                '--python',
                str(venv_python),
                '--no-index',
                '--find-links',
                str(wheelhouse),
            ]
            for spec in INSTALL_PROBE_SPECS
        )
        commands.extend([str(venv_python), '-c', code] for code in INSTALL_PROBE_IMPORTS)

        for command in commands:
            step = run_command(command, env)
            probe['steps'].append(step)
            if not step['ok']:
                probe['ok'] = False
                probe['error'] = step['stderr'] or step['stdout'] or f"command failed: {command}"
                return probe

    probe['ok'] = True
    return probe


def normalized_wheel_name(path: str) -> str:
    name = Path(path).name.lower().replace('-', '_')
    return name


def missing_release_wheels(manifest: dict) -> list[str]:
    wheel_names = [normalized_wheel_name(entry['path']) for entry in manifest.get('wheelhouse', [])]
    missing = []
    for required in sorted(REQUIRED_RELEASE_WHEELS):
        if required == 'paddlepaddle':
            if not any(name.startswith('paddlepaddle') for name in wheel_names):
                missing.append('paddlepaddle_or_paddlepaddle_gpu')
            continue
        if not any(name.startswith(required) for name in wheel_names):
            missing.append(required)
    return missing


def unseeded_cache_markers(pack_root: Path) -> list[str]:
    return sorted(path.relative_to(pack_root).as_posix() for path in pack_root.rglob(CACHE_NOT_SEEDED_MARKER))


def missing_release_cache_dirs(pack_root: Path) -> list[str]:
    missing = []
    for rel in REQUIRED_RELEASE_CACHE_DIRS:
        cache_dir = pack_root / rel
        if not cache_dir.is_dir() or not any(path.is_file() for path in cache_dir.rglob('*')):
            missing.append(rel)
    return missing


def run_smoke(platform: str, root: Path, release: bool = False, install_probe: bool = False) -> dict:
    pack_root = resolve_pack_root(root, platform)
    manifest = load_manifest(root, platform)
    missing = [rel for rel in required_paths(manifest) if not (pack_root / rel).exists()]
    manifest_missing = []
    manifest_mismatched = []
    for entry in iter_manifest_entries(manifest):
        target = pack_root / entry['path']
        if not target.exists():
            manifest_missing.append(entry['path'])
            continue
        if target.stat().st_size != entry['size']:
            manifest_mismatched.append(f"size:{entry['path']}")
        if sha256_file(target) != entry['sha256']:
            manifest_mismatched.append(f"sha256:{entry['path']}")

    contract_errors = []
    python_relpath = manifest.get('python_relpath')
    uv_relpath = manifest.get('uv_relpath')
    if not python_relpath:
        contract_errors.append('missing python_relpath')
    if not uv_relpath:
        contract_errors.append('missing uv_relpath')
    if python_relpath not in {entry['path'] for entry in manifest.get('python_files', [])}:
        contract_errors.append('python_relpath missing from python_files')
    if uv_relpath not in {entry['path'] for entry in manifest.get('uv_files', [])}:
        contract_errors.append('uv_relpath missing from uv_files')

    release_errors = []
    missing_wheels = []
    version_probes = {}
    install_probe_result = None
    if release:
        if manifest.get('payload_profile') != 'release':
            release_errors.append('release smoke requires payload_profile=release')
        if manifest.get('payload_profile') == 'fixture':
            release_errors.append('release smoke cannot use fixture payload_profile')
        if manifest.get('release_injection_required') is not False:
            release_errors.append('release smoke requires release_injection_required=false')
        if manifest.get('external_artifacts_required') != []:
            release_errors.append('release smoke requires external_artifacts_required=[]')
        for key in ('python_files', 'uv_files', 'wheelhouse', 'script_files'):
            if not manifest.get(key):
                release_errors.append(f'release smoke requires non-empty {key}')
        script_paths = {entry['path'] for entry in manifest.get('script_files', [])}
        for script in sorted(REQUIRED_RELEASE_SCRIPTS):
            if script not in script_paths:
                release_errors.append(f'release smoke missing script_files entry: {script}')
        missing_wheels = missing_release_wheels(manifest)
        for wheel in missing_wheels:
            release_errors.append(f'release smoke missing wheelhouse package: {wheel}')
        for cache_dir in missing_release_cache_dirs(pack_root):
            release_errors.append(f'release smoke missing seeded cache directory: {cache_dir}')
        for marker in unseeded_cache_markers(pack_root):
            release_errors.append(f'release smoke found unseeded cache marker: {marker}')
        native_asset_paths = {entry['path'] for entry in manifest.get('native_assets', [])}
        for asset in sorted(REQUIRED_RELEASE_NATIVE_ASSETS):
            if asset not in native_asset_paths:
                release_errors.append(f'release smoke missing native asset entry: {asset}')
            if not (pack_root / asset).is_file():
                release_errors.append(f'release smoke missing native asset file: {asset}')
        if python_relpath and (pack_root / python_relpath).exists():
            version_probes['python'] = run_version_probe(pack_root / python_relpath, platform)
        if uv_relpath and (pack_root / uv_relpath).exists():
            version_probes['uv'] = run_version_probe(pack_root / uv_relpath, platform)
        for name, probe in version_probes.items():
            if probe['attempted'] and not probe['ok']:
                release_errors.append(f'{name} --version failed')
        if install_probe and not release_errors:
            install_probe_result = run_install_probe(pack_root, manifest, platform)
            if install_probe_result['attempted'] and not install_probe_result['ok']:
                release_errors.append('release install probe failed')

    if manifest.get('payload_profile') == 'release' and manifest.get('external_artifacts_required'):
        contract_errors.append('release pack still declares external_artifacts_required')

    return {
        'platform': platform,
        'root': str(pack_root),
        'release': release,
        'payload_profile': manifest.get('payload_profile'),
        'release_injection_required': manifest.get('release_injection_required'),
        'external_artifacts_required': manifest.get('external_artifacts_required', []),
        'entry_counts': {
            key: len(manifest.get(key, []))
            for key in ('python_files', 'uv_files', 'script_files', 'wheelhouse', 'caches', 'native_assets')
        },
        'missing': missing,
        'manifest_missing': manifest_missing,
        'manifest_mismatched': manifest_mismatched,
        'contract_errors': contract_errors,
        'release_errors': release_errors,
        'missing_release_wheels': missing_wheels,
        'version_probes': version_probes,
        'install_probe': install_probe_result,
        'ok': not missing
        and not manifest_missing
        and not manifest_mismatched
        and not contract_errors
        and not release_errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description='Smoke-check EntropIA runtime-pack fixture structure.')
    parser.add_argument('--platform', required=True)
    parser.add_argument(
        '--root',
        default=str(RUNTIME_PACK_ROOT),
        help='Runtime-pack parent directory or the platform-specific assembled directory to inspect.',
    )
    parser.add_argument('--release', action='store_true', help='Enforce release payload hardening checks.')
    parser.add_argument(
        '--install-probe',
        action='store_true',
        help='In release mode, create a temporary offline venv, install core packages from wheelhouse, and import them.',
    )
    args = parser.parse_args()

    result = run_smoke(args.platform, Path(args.root), release=args.release, install_probe=args.install_probe)
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 0 if result['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
