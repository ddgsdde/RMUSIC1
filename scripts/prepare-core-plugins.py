#!/usr/bin/env python3
"""Provision built-in providers before deploying Core; keep all login state in place."""
import argparse, hashlib, json, os, pathlib, re, shutil, subprocess, tempfile, time


def checked(command, **kwargs):
    subprocess.run(command, check=True, timeout=300, **kwargs)


def upgrade_source(source, target):
    """Explicit built-in upgrade; never copy or overwrite plugin account/state files."""
    old = json.loads((target / 'manifest.json').read_text())
    new = json.loads((source / 'manifest.json').read_text())
    if old['id'] != new['id'] or old['id'] != source.name:
        raise ValueError('内置插件身份不匹配，停止更新')
    names = ['provider.py', 'manifest.json', 'requirements.txt']
    for helper in ('qq_artwork.py','public_metadata.py','lyric_timing.py'):
        if (source / helper).is_file(): names.append(helper)
    if all((target / name).is_file() and (target / name).read_bytes() == (source / name).read_bytes() for name in names):
        return
    # Validate the new program with installed dependencies and anonymous, isolated data first.
    executable = target / '.venv/bin/python'
    if (target / 'requirements.txt').read_bytes() != (source / 'requirements.txt').read_bytes():
        raise ValueError('插件依赖发生变化，请先通过插件管理器安装新版本')
    with tempfile.TemporaryDirectory(prefix='rmusic-upgrade-probe-') as data:
        request = json.dumps({'id': 'health', 'method': 'health', 'params': {}}) + '\n'
        result = subprocess.run([str(executable), '-u', str(source / 'provider.py')], input=request, capture_output=True, text=True, timeout=30, cwd=source,
            env=os.environ | {'RMUSIC_PROVIDER_DATA_DIR': data, 'RMUSIC_PROVIDER_ANONYMOUS': '1', 'PYTHONUTF8': '1', 'PYTHONIOENCODING': 'utf-8'})
        if result.returncode or len(result.stdout) > 128 * 1024 or json.loads(result.stdout).get('result', {}).get('status') != 'ready':
            raise RuntimeError('新插件独立健康检查失败，保留原版本')
    backup = target / '.source-backups' / str(time.time_ns())
    backup.mkdir(parents=True)
    for name in names:
        if (target / name).exists(): shutil.copy2(target / name, backup / name)
    try:
        # Manifest is published last, after the implementation is present.
        for name in [name for name in names if name not in {'provider.py','manifest.json'}] + ['provider.py','manifest.json']:
            with tempfile.NamedTemporaryFile(dir=target, prefix='.upgrade-', delete=False) as f:
                temp = pathlib.Path(f.name); f.write((source / name).read_bytes())
            os.replace(temp, target / name)
    except Exception:
        for name in names:
            if (backup / name).exists(): shutil.copy2(backup / name, target / name)
            else: (target / name).unlink(missing_ok=True)
        raise
    print(f'{new["id"]}: 已更新程序 {old["version"]} → {new["version"]}，保留登录状态', flush=True)


def prepare(plugin_root, builtin_root, python, upgrade_builtin=False):
    plugin_root.mkdir(parents=True, exist_ok=True)
    for source in sorted(builtin_root.iterdir()):
        if not (source / 'manifest.json').is_file() or not (source / 'provider.py').is_file():
            continue
        target = plugin_root / source.name
        if not target.exists():
            target.mkdir()
            for file in source.iterdir():
                if file.is_file(): shutil.copy2(file, target / file.name)
        # Existing provider sources and account state are never replaced by deployment.
        manifest = json.loads((target / 'manifest.json').read_text())
        requirements = target / 'requirements.txt'
        if not requirements.is_file(): continue
        raw = requirements.read_bytes()
        if len(raw) > 65536: raise ValueError('requirements.txt 过大')
        for line in raw.decode().splitlines():
            line = line.strip()
            if line and not line.startswith('#') and not re.fullmatch(r'[A-Za-z0-9_.-]+==[A-Za-z0-9_.+-]+', line):
                raise ValueError('插件依赖必须为固定版本的 PyPI 包')
        venv = target / '.venv'
        executable = venv / 'bin/python'
        if not executable.exists(): checked([python, '-m', 'venv', str(venv)])
        stamp = venv / 'rmusic-requirements.sha256'
        digest = hashlib.sha256(raw).hexdigest()
        if not stamp.exists() or stamp.read_text().strip() != digest:
            checked([str(executable), '-m', 'pip', '--isolated', 'install', '--index-url', 'https://pypi.org/simple', '--no-cache-dir', '--disable-pip-version-check', '--no-input', '-r', str(requirements)])
            checked([str(executable), '-m', 'pip', '--isolated', 'check'])
            stamp.write_text(digest)
        # Health uses its own empty data directory, never the user's provider credentials.
        with tempfile.TemporaryDirectory(prefix='rmusic-provider-probe-') as data:
            requests = ''.join(json.dumps({'id': method, 'method': method, 'params': {}}) + '\n' for method in ['manifest', 'health'])
            result = subprocess.run([str(executable), '-u', str(target / 'provider.py')], input=requests, capture_output=True, text=True, timeout=30, cwd=target,
                env=os.environ | {'RMUSIC_PROVIDER_DATA_DIR': data, 'RMUSIC_PROVIDER_ANONYMOUS': '1', 'PYTHONUTF8': '1', 'PYTHONIOENCODING': 'utf-8'})
            if result.returncode or len(result.stdout) > 128 * 1024:
                raise RuntimeError(f'{source.name} 插件启动检查失败（退出码 {result.returncode}）')
            replies = [json.loads(line) for line in result.stdout.splitlines()]
            if len(replies) != 2 or replies[0].get('result', {}).get('id') != manifest['id'] or replies[1].get('result', {}).get('status') != 'ready':
                raise RuntimeError(f'{source.name} 插件健康检查失败')
        if upgrade_builtin:
            upgrade_source(source, target)
            manifest = json.loads((target / 'manifest.json').read_text())
        print(f'{manifest["id"]} {manifest["version"]}: 依赖完整，独立健康检查 ready', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--plugin-root', type=pathlib.Path, required=True)
    parser.add_argument('--builtin-root', type=pathlib.Path, required=True)
    parser.add_argument('--python', required=True)
    parser.add_argument('--upgrade-builtin', action='store_true', help='显式更新内置插件程序，保留账号数据并备份旧程序')
    args = parser.parse_args()
    prepare(args.plugin_root.resolve(), args.builtin_root.resolve(), args.python, args.upgrade_builtin)
