"""Offline administrator backup/restore. Archives contain account and provider secrets."""
import argparse
from contextlib import closing, contextmanager, nullcontext
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path, PurePosixPath, PureWindowsPath
import re
import shutil
import sqlite3
import stat
import tempfile
import zipfile

FORMAT = 1
CORE_VERSION = '0.1.0'
DIRECTORIES = ('providers', 'covers', 'downloads', 'plugins', 'dsp', 'remote-access')
SKIP = {'.venv', '__pycache__', '.staging'}


def portable(name):
    parts = name.split('/')
    reserved = {'CON', 'PRN', 'AUX', 'NUL'} | {f'{p}{i}' for p in ['COM', 'LPT'] for i in range(1, 10)}
    if any(not p or p in {'.', '..'} or p[-1] in ' .' or any(ord(c) < 32 or c in '\\:<>"|?*' for c in p)
           or p.split('.')[0].upper() in reserved for p in parts):
        raise ValueError(f'不可移植或越界的归档路径：{name}')
    if parts[0] != 'data' or (name != 'data/library.db' and (len(parts) < 3 or parts[1] not in DIRECTORIES)):
        raise ValueError(f'归档包含未知数据路径：{name}')
    return parts[1:]


def linked(path):
    return path.is_symlink() or getattr(path, 'is_junction', lambda: False)()


@contextmanager
def offline_lock(data):
    path = data / '.core.lock'
    if linked(path):
        raise ValueError('核心锁文件不能是链接')
    with path.open('a+b') as handle:
        try:
            if os.name == 'nt':
                import msvcrt
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise ValueError('数据目录仍由核心或其他备份任务使用，请先停止它们') from error
        yield handle


def database_info(path):
    with closing(sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)) as db:
        if db.execute('PRAGMA quick_check').fetchall() != [('ok',)] or db.execute('PRAGMA foreign_key_check').fetchone():
            raise ValueError('数据库完整性检查失败')
        tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        if not {'users', 'settings', 'roots', 'tracks', 'zones', 'playlists', 'track_stats'}.issubset(tables):
            raise ValueError('不是支持的 R-Music 核心数据库')
        identity = json.loads(db.execute("SELECT value FROM settings WHERE key='core_id'").fetchone()[0])
        roots = [dict(id=r[0], path=r[1]) for r in db.execute('SELECT id,path FROM roots ORDER BY id')]
        return dict(core_id=identity, roots=roots)


def files(directory):
    if not directory.exists():
        return
    if linked(directory):
        raise ValueError(f'备份数据目录不能是链接：{directory}')
    for folder, dirs, names in os.walk(directory, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d not in SKIP and not d.startswith('.health-')
                         and not (directory.name == 'remote-access' and d.startswith('request-')))
        for name in dirs + names:
            path = Path(folder) / name
            if linked(path):
                raise ValueError(f'备份不能跟随链接：{path}')
        for name in sorted(names):
            path = Path(folder) / name
            if name.endswith(('.pyc', '.partial')) or (directory.name == 'remote-access' and name.endswith('.next')):
                continue
            if not path.is_file():
                raise ValueError(f'不支持的数据文件：{path}')
            yield path


def copy_stream(source, target=None):
    digest = hashlib.sha256(); size = 0
    while chunk := source.read(1024 * 1024):
        digest.update(chunk); size += len(chunk)
        if target is not None:
            target.write(chunk)
    return dict(size=size, sha256=digest.hexdigest())


def backup(data, output, plugins=None, *, online=False):
    original_data = Path(data).absolute()
    data = original_data.resolve(strict=True); output = Path(output).absolute()
    plugins = Path(plugins).resolve(strict=True) if plugins else data / 'plugins'
    if not online and (output.resolve().is_relative_to(data) or output.resolve().is_relative_to(plugins)):
        raise ValueError('备份文件须保存在核心／插件数据目录之外')
    if linked(data / 'library.db'):
        raise ValueError('核心数据库不能是链接')
    with (nullcontext(None) if online else offline_lock(data)) as lock, tempfile.TemporaryDirectory(prefix='rmusic-backup-') as temp:
        if lock is not None:
            lock.seek(0)
        core_version = CORE_VERSION if online else json.loads(lock.read() or b'{}').get('core_version')
        snapshot = Path(temp) / 'library.db'
        with closing(sqlite3.connect((data / 'library.db').as_uri() + '?mode=ro', uri=True)) as source, closing(sqlite3.connect(snapshot)) as target:
            source.backup(target, pages=64, sleep=0.01)
            target.execute('PRAGMA journal_mode=DELETE')
        info = database_info(snapshot)
        candidates = [('data/library.db', snapshot)]
        for name in (('providers', 'covers', 'dsp', 'remote-access') if online else DIRECTORIES):
            directory = plugins if name == 'plugins' else data / name
            candidates.extend((f'data/{name}/{p.relative_to(directory).as_posix()}', p) for p in files(directory))
        names = [name.casefold() for name, _ in candidates]
        if len(set(names)) != len(names):
            raise ValueError('文件名在大小写不敏感的平台上冲突')
        for name, _ in candidates:
            portable(name)
        # Exclusive creation preserves any existing archive; incomplete output is removed on failure.
        descriptor = os.open(output, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
        try:
            entries = []; observed = []
            with os.fdopen(descriptor, 'w+b') as raw, zipfile.ZipFile(raw, 'w', zipfile.ZIP_DEFLATED, compresslevel=3, allowZip64=True) as archive:
                for name, path in candidates:
                    with path.open('rb') as source, archive.open(name, 'w', force_zip64=True) as target:
                        before = os.fstat(source.fileno())
                        entries.append(dict(path=name, **copy_stream(source, target)))
                        after = os.fstat(source.fileno())
                        if (after.st_size, after.st_mtime_ns) != (before.st_size, before.st_mtime_ns):
                            raise ValueError('备份期间数据发生变化，文件正在写入，请稍后重试')
                    observed.append((path, before.st_size, before.st_mtime_ns))
                if not online and any((p.stat().st_size, p.stat().st_mtime_ns) != (size, modified) for p, size, modified in observed):
                    raise ValueError('备份期间数据发生变化，请停止核心及音源进程后重试')
                manifest = dict(application='R-Music', format=FORMAT, core_version=core_version, tool_version=CORE_VERSION,
                                created_at=datetime.now(timezone.utc).isoformat(), source_platform=os.name,
                                data_path=str(original_data), contains_secrets=True, files=entries, kind='database' if online else 'full', **info)
                archive.writestr('manifest.json', json.dumps(manifest, ensure_ascii=False))
            return manifest
        except BaseException:
            output.unlink(missing_ok=True)
            raise


def unpack(archive_path, destination, only_database=False):
    with zipfile.ZipFile(archive_path) as archive:
        members = archive.infolist()
        if len({m.filename.casefold() for m in members}) != len(members):
            raise ValueError('归档包含重复或大小写冲突的路径')
        metadata = archive.getinfo('manifest.json')
        manifest = json.loads(archive.read(metadata))
        if not isinstance(manifest, dict) or manifest.get('application') != 'R-Music' or type(manifest.get('format')) is not int or manifest['format'] != FORMAT:
            raise ValueError('不支持的备份格式')
        version = manifest.get('core_version')
        if version is not None and (not isinstance(version, str) or not re.fullmatch(r'\d+\.\d+\.\d+', version) or tuple(map(int, version.split('.'))) > tuple(map(int, CORE_VERSION.split('.')))):
            raise ValueError('备份来自较新的或未知版本核心，请先升级恢复工具')
        entries = manifest['files']
        if len({e['path'].casefold() for e in entries}) != len(entries) or {e['path'] for e in entries} != {m.filename for m in members if m.filename != 'manifest.json'}:
            raise ValueError('归档与备份清单不一致')
        if sum(e['size'] for e in entries) > shutil.disk_usage(destination).free and not only_database:
            raise ValueError('恢复目录磁盘空间不足')
        for entry in entries:
            parts = portable(entry['path']); member = archive.getinfo(entry['path'])
            mode = member.external_attr >> 16
            if member.is_dir() or stat.S_ISLNK(mode) or (stat.S_IFMT(mode) not in [0, stat.S_IFREG]) or member.flag_bits & 1:
                raise ValueError('归档只允许普通、未加密的数据文件')
            if type(entry['size']) is not int or entry['size'] < 0 or member.file_size != entry['size'] or not re.fullmatch('[0-9a-f]{64}', entry['sha256']):
                raise ValueError('无效的文件完整性清单')
            output = destination.joinpath(*parts)
            write = not only_database or entry['path'] == 'data/library.db'
            if write:
                output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                with archive.open(member) as source, output.open('xb') as target:
                    os.chmod(output, 0o600)
                    measured = copy_stream(source, target)
            else:
                with archive.open(member) as source:
                    measured = copy_stream(source)
            if measured != {k:entry[k] for k in ['size', 'sha256']}:
                raise ValueError(f'备份文件校验失败：{entry["path"]}')
        info = database_info(destination / 'library.db')
        if info['core_id'] != manifest['core_id'] or info['roots'] != manifest['roots']:
            raise ValueError('数据库身份／曲库目录与清单不一致')
        return manifest


def verify(archive):
    with tempfile.TemporaryDirectory(prefix='rmusic-verify-') as temp:
        return unpack(archive, Path(temp), only_database=True)


def rehearse(archive):
    """Restore into disposable storage, including path migration and safe stopped transport."""
    with tempfile.TemporaryDirectory(prefix='rmusic-rehearsal-') as temp:
        stage = Path(temp)
        manifest = verify(archive)
        mappings = [(r['path'], str(stage / f'music-{i}')) for i, r in enumerate(manifest['roots'])]
        target = stage / 'restored'
        result = restore(archive, target, mappings)
        with closing(sqlite3.connect(target / 'library.db')) as db:
            if any(json.loads(raw).get('status') != 'stopped' for raw, in db.execute('SELECT data FROM zones')):
                raise ValueError('恢复后输出未停止')
        return result


def old_path(value, platform):
    if platform == 'nt':
        if value.startswith('\\\\?\\UNC\\'):
            value = '\\\\' + value[8:]
        elif value.startswith('\\\\?\\'):
            value = value[4:]
        return PureWindowsPath(value)
    return PurePosixPath(value)


def canonical(path):
    value = str(path.resolve())
    if os.name == 'nt' and not value.startswith('\\\\?\\'):
        return '\\\\?\\UNC\\' + value[2:] if value.startswith('\\\\') else '\\\\?\\' + value
    return value


def migrate(stage, target, manifest, mappings):
    platform = manifest['source_platform']
    if platform not in ['nt', 'posix']:
        raise ValueError('未知的原平台路径格式')
    replacements = {}
    for old, new in mappings:
        key = old_path(old, platform)
        if key in replacements or not Path(new).is_absolute():
            raise ValueError('迁移目录须唯一映射到本机绝对路径')
        replacements[key] = canonical(Path(new))
    roots = {old_path(r['path'], platform) for r in manifest['roots']}
    if replacements.keys() - roots:
        raise ValueError('映射中存在备份未包含的曲库目录')
    if platform != os.name and roots - replacements.keys():
        raise ValueError('跨平台恢复必须显式映射全部音乐目录')
    with closing(sqlite3.connect(stage / 'library.db')) as db:
        db.execute('PRAGMA foreign_keys=ON')
        for root in manifest['roots']:
            old = old_path(root['path'], platform)
            if old not in replacements:
                continue
            new = replacements[old]
            rows = db.execute('SELECT id,path FROM tracks WHERE root_id=? AND path IS NOT NULL', [root['id']]).fetchall()
            for track, path in rows:
                relative = old_path(path, platform).relative_to(old)
                if '..' in relative.parts:
                    raise ValueError('曲目路径越过原音乐目录')
                db.execute('UPDATE tracks SET path=? WHERE id=?', [str(Path(new).joinpath(*relative.parts)), track])
            db.execute('UPDATE roots SET path=? WHERE id=?', [new, root['id']])
        original = old_path(manifest['data_path'], platform)
        for download, path in db.execute('SELECT id,path FROM downloads WHERE path IS NOT NULL').fetchall():
            relative = old_path(path, platform).relative_to(original)
            if not relative.parts or relative.parts[0] != 'downloads' or '..' in relative.parts:
                raise ValueError('下载记录路径越过原数据目录')
            saved = stage.joinpath(*relative.parts)
            if saved.is_file():
                db.execute('UPDATE downloads SET path=? WHERE id=?', [str(target.joinpath(*relative.parts)), download])
            else:
                db.execute("UPDATE downloads SET path=NULL,status='interrupted',error='备份未包含完整下载文件' WHERE id=?", [download])
        for zone_id, raw in db.execute('SELECT id,data FROM zones').fetchall():
            zone = json.loads(raw)
            if zone.get('playback_id') and zone.get('track'):
                active = db.execute('SELECT last_position FROM play_sessions WHERE zone_id=? AND track_id=? AND finished_at IS NULL ORDER BY last_report_at DESC,rowid DESC LIMIT 1', [zone_id, zone['track']['id']]).fetchone()
                if active:
                    zone['position'] = active[0]
            zone.update(status='stopped', playback_id=None)
            db.execute('UPDATE zones SET data=? WHERE id=?', [json.dumps(zone, ensure_ascii=False), zone_id])
        tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        for table in ['sequence_recovery_receipts', 'sequence_recovery', 'sequence_receipts', 'cache_exports', 'cache_entries']:
            if table in tables:
                db.execute(f'DELETE FROM {table}')
        db.execute("DELETE FROM settings WHERE key IN ('cache_directory','backup_policy')")
        if 'maintenance_backups' in tables:
            db.execute("UPDATE maintenance_backups SET status='missing',error='恢复后请重新选择备份目录'")
        if 'file_bin' in tables:
            db.execute("UPDATE file_bin SET status='missing',error='备份不包含回收站音乐文件' WHERE status IN ('planned','trashed','restoring')")
        db.execute("UPDATE tracks SET available=0 WHERE source='local' AND json_type(provider_data,'$.cached_from.cache_id')='text'")
        db.execute("UPDATE jobs SET status='interrupted',cursor=NULL WHERE status IN ('running','paused','pending')")
        db.execute("UPDATE downloads SET status='interrupted',error='恢复后请重新下载' WHERE status='downloading'")
        db.commit()
        db.execute('PRAGMA wal_checkpoint(TRUNCATE)')
    database_info(stage / 'library.db')


def restore(archive, target, mappings=()):
    target = Path(target).absolute()
    if target.exists() or linked(target):
        raise ValueError('恢复目标必须是尚不存在的新目录；原数据不会被覆盖')
    parent = target.parent.resolve(strict=True)
    target = parent / target.name
    stage = Path(tempfile.mkdtemp(prefix='.rmusic-restore-', dir=parent))
    try:
        manifest = unpack(archive, stage)
        migrate(stage, target, manifest, mappings)
        if target.exists() or linked(target):
            raise ValueError('恢复目标已被其他任务创建')
        stage.rename(target)
        return {**manifest, **database_info(target / 'library.db')}
    finally:
        # Only this invocation's freshly allocated staging directory is eligible for cleanup.
        if stage.exists() and stage.parent == parent and stage.name.startswith('.rmusic-restore-') and not linked(stage):
            shutil.rmtree(stage)


def online_backup(data, output):
    output = Path(output).absolute()
    if output.exists() or linked(output):
        raise ValueError('备份目标已存在，不覆盖已有备份')
    partial = output.with_name(output.name + '.partial')
    if partial.exists():
        if linked(partial) or not partial.is_file():
            raise ValueError('备份暂存路径不是普通文件')
        partial.unlink()  # This exact job's unpublished file, never a music/cache file.
    try:
        result = backup(data, partial, online=True)
        verify(partial)
        with partial.open('r+b') as file:
            os.fsync(file.fileno())
        if output.exists():
            raise ValueError('备份目标已存在')
        os.link(partial, output)  # atomic, no-replace on a single filesystem
        partial.unlink()
        if os.name != 'nt':
            parent_fd = os.open(output.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(parent_fd)
            finally:
                os.close(parent_fd)
        return result
    finally:
        if partial.is_file() and not linked(partial):
            partial.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    create = commands.add_parser('create', help='停止核心及音源进程后备份；含账号密钥和提供方登录态')
    create.add_argument('--data-dir', required=True); create.add_argument('--output', required=True)
    create.add_argument('--plugins-dir', default=os.environ.get('RMUSIC_PLUGIN_DIR'))
    create.add_argument('--offline', action='store_true', required=True, help='确认已停止核心及其音源进程；新版核心另有目录锁校验')
    online = commands.add_parser('create-database', help='在线SQLite快照及音源登录／封面／DSP；不含音乐、下载或插件程序')
    online.add_argument('--data-dir', required=True); online.add_argument('--output', required=True)
    drill = commands.add_parser('rehearse', help='在临时新目录执行真实恢复演练，原数据不变')
    drill.add_argument('archive')
    check = commands.add_parser('verify', help='校验完整归档和 SQLite，不打印凭据')
    check.add_argument('archive')
    recover = commands.add_parser('restore', help='校验后恢复到一个新目录')
    recover.add_argument('archive'); recover.add_argument('--data-dir', required=True)
    recover.add_argument('--map-root', nargs=2, action='append', default=[], metavar=('OLD', 'NEW'))
    args = parser.parse_args()
    try:
        if args.command == 'create':
            result = backup(args.data_dir, args.output, args.plugins_dir)
        elif args.command == 'create-database':
            result = online_backup(args.data_dir, args.output)
        elif args.command == 'rehearse':
            result = rehearse(args.archive)
        elif args.command == 'verify':
            result = verify(args.archive)
        else:
            result = restore(args.archive, args.data_dir, args.map_root)
        print(json.dumps(dict(ok=True, command=args.command, core_id=result['core_id'], roots=result['roots'],
                              files=len(result['files']), bytes=sum(e['size'] for e in result['files']), contains_secrets=True), ensure_ascii=False))
    except (OSError, ValueError, KeyError, TypeError, sqlite3.Error, zipfile.BadZipFile) as error:
        parser.exit(1, f'备份／恢复失败：{error}\n')


if __name__ == '__main__':
    main()
