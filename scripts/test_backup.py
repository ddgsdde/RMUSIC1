from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch
import zipfile

import backup


class BackupTest(unittest.TestCase):
    def test_online_backup_restore_removes_quality_rows_with_omitted_audio_cache(self):
        with closing(sqlite3.connect(self.data/'library.db')) as db:
            db.execute("INSERT INTO cache_entries(id,track_id,format,last_access,path)VALUES('cached','stable-track','flac',1,'/cache/audio.flac')")
            db.execute("INSERT INTO cache_quality VALUES('cached','{}')")
            db.commit()
        backup.online_backup(self.data,self.archive)
        backup.rehearse(self.archive)
        target=self.root/'restored-cache'
        backup.restore(self.archive,target)
        with closing(sqlite3.connect(target/'library.db')) as db:
            self.assertEqual(db.execute('SELECT COUNT(*) FROM cache_entries').fetchone()[0],0)
            self.assertEqual(db.execute('SELECT COUNT(*) FROM cache_quality').fetchone()[0],0)
            self.assertEqual(db.execute('PRAGMA foreign_key_check').fetchall(),[])

    def test_online_backup_keeps_the_open_snapshot_when_a_state_file_is_atomically_replaced(self):
        state=self.data/'providers'/'session.json';state.parent.mkdir(exist_ok=True);state.write_text('old complete session')
        original=backup.copy_stream
        def replace(source,target=None):
            result=original(source,target)
            if Path(source.name).resolve()==state.resolve():
                staged=state.with_suffix('.next');staged.write_text('new complete session');staged.replace(state)
            return result
        with patch.object(backup,'copy_stream',side_effect=replace):
            backup.online_backup(self.data,self.archive)
        with zipfile.ZipFile(self.archive) as archive:
            self.assertEqual(archive.read('data/providers/session.json'),b'old complete session')
        self.assertEqual(state.read_text(),'new complete session')
        backup.verify(self.archive)

    def test_remote_identity_and_revocations_survive_restore_without_transient_secrets(self):
        root = self.data / 'remote-access'
        (root / 'secrets').mkdir(parents=True)
        (root / 'secrets/vault.key').write_bytes(bytes(range(32)))
        (root / 'secrets/p2p-devices.sealed').write_bytes(b'sealed fixture')
        (root / 'request-test').mkdir()
        (root / 'request-test/dns-token').write_text('temporary fixture')
        backup.online_backup(self.data, self.archive)
        target = self.root / 'restored-network'
        backup.restore(self.archive, target)
        key = target / 'remote-access/secrets/vault.key'
        self.assertEqual(key.read_bytes(), bytes(range(32)))
        if os.name != 'nt':
            self.assertEqual(key.stat().st_mode & 0o077, 0)
        self.assertEqual((key.parent / 'p2p-devices.sealed').read_bytes(), b'sealed fixture')
        self.assertFalse((target / 'remote-access/request-test').exists())

    def test_catalog_image_budget_and_identity_history_survive_restore(self):
        with closing(sqlite3.connect(self.data / 'library.db')) as db:
            db.executemany('INSERT INTO settings VALUES(?,?)', [('artwork_limit_bytes','536870912'),('catalog_scrape_enabled','false'),('catalog_scrape_concurrency','1')])
            db.execute("INSERT INTO provider_generations VALUES('fixture',7)")
            db.execute("INSERT INTO artwork_sources(id,url,cover_id)VALUES('source','https://fixture.example/image','missing-cover')")
            db.execute("INSERT INTO artwork_versions VALUES('missing-cover','source')")
            db.execute("INSERT INTO artwork_health VALUES('source','missing','not_found',1)")
            db.execute("INSERT INTO catalog_profiles(kind,entity_id,automatic,manual,revision)VALUES('artist','artist-id','{\"name\":\"Fixture\"}','{\"description\":\"manual biography\"}',3)")
            db.execute("INSERT INTO music_identity_audit(track_id,revision,user_id,before_data,after_data,created_at)VALUES('stable-track',1,'u','{}','{}',1)")
            db.commit()
        self.create();target=self.root/'restored-catalog';backup.restore(self.archive,target)
        with closing(sqlite3.connect(target/'library.db')) as db:
            self.assertEqual(db.execute("SELECT value FROM settings WHERE key='artwork_limit_bytes'").fetchone()[0],'536870912')
            self.assertEqual(db.execute("SELECT revision FROM provider_generations WHERE provider='fixture'").fetchone()[0],7)
            self.assertEqual(db.execute("SELECT source_id FROM artwork_versions WHERE cover_id='missing-cover'").fetchone()[0],'source')
            self.assertEqual(json.loads(db.execute("SELECT manual FROM catalog_profiles WHERE entity_id='artist-id'").fetchone()[0])['description'],'manual biography')
            self.assertEqual(db.execute("SELECT COUNT(*) FROM music_identity_audit").fetchone()[0],1)
            self.assertEqual(db.execute("SELECT tracks FROM playlists WHERE id='p'").fetchone()[0],'["stable-track"]')

    def test_tool_version_matches_the_core_release_version(self):
        with (Path(__file__).parents[1] / 'Cargo.toml').open('rb') as stream:
            self.assertEqual(backup.CORE_VERSION, tomllib.load(stream)['workspace']['package']['version'])

    def test_impulse_assets_restore_byte_exact_and_abandoned_staging_is_excluded(self):
        assets=self.data/'dsp/impulses';assets.mkdir(parents=True)
        (assets/'response.wav').write_bytes(b'synthetic-f64-response-original-bytes')
        (assets/'response.json').write_text('{"name":"Room","sample_rate":48000}',encoding='utf-8')
        (assets/'.staging').mkdir();(assets/'.staging/abandoned.partial').write_bytes(b'uncommitted')
        self.create();target=self.root/'restored-impulses';backup.restore(self.archive,target,[])
        self.assertEqual((target/'dsp/impulses/response.wav').read_bytes(),(assets/'response.wav').read_bytes())
        self.assertEqual((target/'dsp/impulses/response.json').read_bytes(),(assets/'response.json').read_bytes())
        self.assertFalse((target/'dsp/impulses/.staging').exists())

    def test_online_snapshot_preserves_credentials_and_committed_wal(self):
        with closing(sqlite3.connect(self.data/'library.db')) as connection:
            connection.execute('PRAGMA journal_mode=WAL')
            connection.execute("UPDATE track_stats SET play_seconds=84 WHERE track_id='stable-track'")
            connection.commit()
            archive=self.data/'backups'/'online.zip'
            archive.parent.mkdir()
            manifest=backup.online_backup(self.data,archive)
            self.assertEqual(manifest['kind'],'database')
            self.assertTrue(backup.verify(archive)['contains_secrets'])
            with zipfile.ZipFile(archive) as contents:
                self.assertEqual(contents.read('data/providers/qqmusic/credential.json'),self.secret)
                self.assertFalse(any('/downloads/' in name or '/cache/' in name for name in contents.namelist()))
            restored=self.root/'online-restored'
            backup.restore(archive,restored,[])
            with closing(sqlite3.connect(restored/'library.db')) as snapshot:
                self.assertEqual(snapshot.execute("SELECT play_seconds FROM track_stats WHERE track_id='stable-track'").fetchone()[0],84)
            self.assertEqual((restored/'providers/qqmusic/credential.json').read_bytes(),self.secret)
            self.assertFalse(Path(str(archive)+'.partial').exists())

    def test_rehearsal_restores_and_preserves_original_database_and_logins(self):
        self.create()
        before = backup.database_info(self.data / 'library.db')
        result = backup.rehearse(self.archive)
        self.assertEqual(result['core_id'], before['core_id'])
        self.assertEqual(backup.database_info(self.data / 'library.db'), before)
        self.assertEqual((self.data / 'providers/qqmusic/credential.json').read_bytes(), self.secret)
        with closing(sqlite3.connect(self.data / 'library.db')) as db:
            self.assertEqual(json.loads(db.execute('SELECT data FROM zones').fetchone()[0])['status'], 'playing')

    def test_failed_online_write_never_publishes_a_partial_archive(self):
        output = self.root / 'failed.zip'
        with patch.object(backup, 'copy_stream', side_effect=OSError(28, 'No space left on device')):
            with self.assertRaises(OSError):
                backup.online_backup(self.data, output)
        self.assertFalse(output.exists())
        self.assertFalse(Path(str(output) + '.partial').exists())
        backup.online_backup(self.data, output)
        self.assertTrue(backup.verify(output)['contains_secrets'])

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='rmusic-backup-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.data = self.root / 'core'; self.data.mkdir()
        self.music = self.root / 'music'; self.music.mkdir()
        self.archive = self.root / 'backup.zip'
        for folder in ['providers/qqmusic', 'plugins/example/.venv', 'plugins/.rollback/example/old', 'covers', 'downloads', 'cache']:
            (self.data / folder).mkdir(parents=True, exist_ok=True)
        self.secret = b'{"credential":"synthetic-private-login"}'
        (self.data / 'providers/qqmusic/credential.json').write_bytes(self.secret)
        (self.data / 'plugins/example/provider.py').write_text('# synthetic provider', encoding='utf-8')
        (self.data / 'plugins/example/.venv/python.exe').write_bytes(b'not portable')
        (self.data / 'plugins/.rollback/example/old/provider.py').write_text('# prior version', encoding='utf-8')
        (self.data / 'covers/art.jpg').write_bytes(b'cover')
        (self.data / 'downloads/saved.wav').write_bytes(b'complete-download')
        (self.data / 'downloads/new.partial').write_bytes(b'incomplete')
        (self.data / 'cache/audio.bin').write_bytes(b'rebuildable-cache')
        with closing(sqlite3.connect(self.data / 'library.db')) as db:
            db.executescript((Path(__file__).parents[1] / 'crates/core/src/schema.sql').read_text(encoding='utf-8'))
            db.execute("INSERT INTO users(id,username,password,role,totp_secret,recovery) VALUES('u','admin','opaque-password-hash','admin','fake-totp-secret','[\"fake-recovery-hash\"]')")
            db.execute("INSERT INTO sessions VALUES('fake-token-hash','u',9999999999,'API','[\"control\"]',1)")
            db.executemany('INSERT INTO settings VALUES(?,?)', [('core_id', json.dumps('a09d5ea6-3895-4e8e-9a14-3739456484bc')), ('cache_directory', json.dumps(str(self.root / 'external-cache')))])
            db.execute("INSERT INTO roots VALUES('r',?)", [str(self.music)])
            db.execute("INSERT INTO tracks(id,path,root_id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES('stable-track',?,'r','Title','Artist','Album','Artist','local','wav','local:stable-track',1)", [str(self.music / 'song.wav')])
            db.execute("INSERT INTO track_stats(track_id,play_seconds) VALUES('stable-track',42)")
            db.execute("INSERT INTO playlists(id,name,user_id,tracks) VALUES('p','Saved','u','[\"stable-track\"]')")
            db.execute("INSERT INTO downloads(id,track_id,status,size,path,created_at) VALUES('d','stable-track','complete',17,?,1)", [str(self.data / 'downloads/saved.wav')])
            zone = dict(id='z', status='playing', playback_id='live', position=0, track={'id':'stable-track'})
            db.execute('INSERT INTO zones VALUES(?,?)', ['z', json.dumps(zone)])
            db.execute("INSERT INTO sequence_recovery VALUES('z','{}')")
            db.commit()

    def create(self):
        return backup.backup(self.data, self.archive)

    def rewrite(self, change):
        with zipfile.ZipFile(self.archive) as source:
            payload = {m.filename:source.read(m) for m in source.infolist()}
        manifest = json.loads(payload['manifest.json'])
        change(payload, manifest)
        payload['manifest.json'] = json.dumps(manifest).encode()
        with zipfile.ZipFile(self.archive, 'w', zipfile.ZIP_DEFLATED) as target:
            for name, data in payload.items():
                target.writestr(name, data)

    def test_snapshot_includes_wal_accounts_providers_and_portable_plugin_history(self):
        with closing(sqlite3.connect(self.data / 'library.db')) as writer:
            writer.execute('PRAGMA wal_autocheckpoint=0')
            writer.execute("INSERT INTO settings VALUES('wal-only','true')"); writer.commit()
            self.assertTrue((self.data / 'library.db-wal').exists())
            manifest = self.create()
        self.assertTrue(manifest['contains_secrets'])
        self.assertEqual(backup.verify(self.archive)['core_id'], manifest['core_id'])
        names = {e['path'] for e in manifest['files']}
        self.assertIn('data/plugins/.rollback/example/old/provider.py', names)
        self.assertNotIn('data/cache/audio.bin', names)
        self.assertFalse(any('.venv' in n or n.endswith('.partial') for n in names))
        with zipfile.ZipFile(self.archive) as archive:
            self.assertEqual(archive.read('data/providers/qqmusic/credential.json'), self.secret)
            extracted = self.root / 'snapshot.db'; extracted.write_bytes(archive.read('data/library.db'))
        with closing(sqlite3.connect(extracted)) as db:
            self.assertEqual(db.execute("SELECT value FROM settings WHERE key='wal-only'").fetchone(), ('true',))
            self.assertEqual(db.execute('SELECT password,totp_secret,recovery FROM users').fetchone(), ('opaque-password-hash','fake-totp-secret','["fake-recovery-hash"]'))

    def test_restore_remaps_paths_but_preserves_identity_playlists_history_and_credentials(self):
        self.create(); target = self.root / 'restored'; moved = self.root / 'new-music'; moved.mkdir()
        result = backup.restore(self.archive, target, [(str(self.music), str(moved))])
        self.assertEqual(result['roots'][0]['path'], backup.canonical(moved))
        self.assertEqual((target / 'providers/qqmusic/credential.json').read_bytes(), self.secret)
        with closing(sqlite3.connect(target / 'library.db')) as db:
            self.assertEqual(db.execute('SELECT id,path FROM tracks').fetchone(), ('stable-track', str(Path(backup.canonical(moved)) / 'song.wav')))
            self.assertEqual(db.execute('SELECT tracks FROM playlists').fetchone()[0], '["stable-track"]')
            self.assertEqual(db.execute('SELECT play_seconds FROM track_stats').fetchone()[0], 42)
            self.assertEqual(db.execute('SELECT path FROM downloads').fetchone()[0], str(target.resolve() / 'downloads/saved.wav'))
            self.assertEqual(db.execute('SELECT COUNT(*) FROM sequence_recovery').fetchone()[0], 0)
            self.assertIsNone(db.execute("SELECT value FROM settings WHERE key='cache_directory'").fetchone())
            zone = json.loads(db.execute('SELECT data FROM zones').fetchone()[0])
            self.assertEqual(zone['status'], 'stopped'); self.assertIsNone(zone['playback_id'])

    def test_locked_directory_is_rejected_in_another_process(self):
        program = 'import backup,sys;from pathlib import Path\nwith backup.offline_lock(Path(sys.argv[1])):\n print("locked",flush=True);sys.stdin.readline()'
        proc = subprocess.Popen([sys.executable, '-c', program, str(self.data)], cwd=Path(__file__).parent, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        try:
            self.assertEqual(proc.stdout.readline().strip(), 'locked')
            with self.assertRaisesRegex(ValueError, '仍由核心'):
                self.create()
            self.assertFalse(self.archive.exists())
        finally:
            proc.communicate('\n', timeout=5)

    def test_existing_archive_and_restore_directory_are_never_overwritten(self):
        self.archive.write_bytes(b'original')
        with self.assertRaises(FileExistsError): self.create()
        self.assertEqual(self.archive.read_bytes(), b'original')
        self.archive.unlink(); self.create()
        target = self.root / 'existing'; target.mkdir(); (target / 'keep').write_bytes(b'existing')
        with self.assertRaises(ValueError): backup.restore(self.archive, target)
        self.assertEqual((target / 'keep').read_bytes(), b'existing')

    def test_corrupt_payload_does_not_publish_partial_restore(self):
        self.create()
        self.rewrite(lambda payload, manifest:payload.update({'data/providers/qqmusic/credential.json':b'altered'}))
        target = self.root / 'bad'
        with self.assertRaises(ValueError): backup.restore(self.archive, target)
        self.assertFalse(target.exists()); self.assertFalse(list(self.root.glob('.rmusic-restore-*')))

    def test_traversal_reserved_names_and_unlisted_files_are_rejected(self):
        for name in ['data/providers/../../outside', '/data/providers/a', 'data/providers/C:ads', 'data/providers/NUL', 'data/providers/final.', 'data/providers/a\\b']:
            with self.subTest(name=name):
                if self.archive.exists(): self.archive.unlink()
                self.create()
                def extra(payload, manifest):
                    payload[name] = b'x'; manifest['files'].append(dict(path=name,size=1,sha256=hashlib.sha256(b'x').hexdigest()))
                self.rewrite(extra)
                with self.assertRaises(ValueError): backup.restore(self.archive, self.root / 'bad')
                self.assertFalse((self.root / 'bad').exists())
        self.archive.unlink(); self.create()
        self.rewrite(lambda payload, manifest:payload.update({'unlisted':b'x'}))
        with self.assertRaises(ValueError): backup.verify(self.archive)

    def test_symlink_member_is_not_extracted(self):
        self.create()
        with zipfile.ZipFile(self.archive) as archive:
            payload = {m.filename:archive.read(m) for m in archive.infolist()}
        with zipfile.ZipFile(self.archive, 'w') as archive:
            for name, data in payload.items():
                member = zipfile.ZipInfo(name)
                if name.endswith('credential.json'): member.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(member, data)
        with self.assertRaises(ValueError): backup.restore(self.archive, self.root / 'bad')

    def test_unknown_format_and_case_aliases_are_rejected(self):
        self.create(); self.rewrite(lambda payload, manifest:manifest.update(format=99))
        with self.assertRaises(ValueError): backup.verify(self.archive)
        self.archive.unlink(); self.create()
        self.rewrite(lambda payload, manifest:payload.update({'DATA/LIBRARY.DB':payload['data/library.db']}))
        with self.assertRaises(ValueError): backup.verify(self.archive)

    def test_changed_file_aborts_backup_and_removes_only_new_output(self):
        original = backup.copy_stream
        def change(source, target=None):
            result = original(source, target)
            if str(getattr(source, 'name', '')).endswith('credential.json'):
                (self.data / 'providers/qqmusic/credential.json').write_bytes(b'updated-credential')
            return result
        with patch.object(backup, 'copy_stream', side_effect=change):
            with self.assertRaisesRegex(ValueError, '数据发生变化'): self.create()
        self.assertFalse(self.archive.exists())

    def test_windows_extended_paths_are_compared_as_source_paths(self):
        self.assertEqual(backup.old_path('\\\\?\\C:\\Music\\Track.wav', 'nt'), backup.old_path('c:\\music\\track.wav', 'nt'))
        self.assertEqual(str(backup.old_path('\\\\?\\UNC\\server\\music\\track.wav', 'nt')), '\\\\server\\music\\track.wav')

    def test_live_position_is_retained_but_old_playback_authority_is_revoked(self):
        with closing(sqlite3.connect(self.data / 'library.db')) as db:
            db.execute("INSERT INTO play_sessions(id,zone_id,track_id,title,artist,duration,started_at,last_report_at,last_position,last_status,runtime_id) VALUES('s','z','stable-track','Title','Artist',30,1,2,7.25,'playing','runtime')")
            db.commit()
        self.create(); target = self.root / 'restored'; backup.restore(self.archive, target)
        with closing(sqlite3.connect(target / 'library.db')) as db:
            zone = json.loads(db.execute('SELECT data FROM zones').fetchone()[0])
            self.assertEqual(zone['position'], 7.25); self.assertEqual(zone['status'], 'stopped')
            self.assertIsNone(zone['playback_id'])
            self.assertEqual(db.execute('SELECT COUNT(*) FROM sequence_recovery').fetchone()[0], 0)

    def test_cross_platform_missing_mapping_and_newer_core_are_rejected(self):
        self.create(); self.rewrite(lambda payload, manifest:manifest.update(source_platform='posix' if os.name == 'nt' else 'nt'))
        target = self.root / 'bad'
        with self.assertRaisesRegex(ValueError, '显式映射'): backup.restore(self.archive, target)
        self.assertFalse(target.exists())
        self.archive.unlink(); self.create(); self.rewrite(lambda payload, manifest:manifest.update(core_version='9.0.0'))
        with self.assertRaisesRegex(ValueError, '先升级'): backup.verify(self.archive)

    def test_external_plugin_directory_is_included_as_portable_packages(self):
        external = self.root / 'external-plugins'
        shutil.move(self.data / 'plugins', external)
        backup.backup(self.data, self.archive, external)
        target = self.root / 'restored'; backup.restore(self.archive, target)
        self.assertEqual((target / 'plugins/example/provider.py').read_text(), '# synthetic provider')
        self.assertFalse((target / 'plugins/example/.venv').exists())

    def test_missing_download_is_marked_interrupted_instead_of_complete(self):
        (self.data / 'downloads/saved.wav').unlink(); self.create()
        target = self.root / 'restored'; backup.restore(self.archive, target)
        with closing(sqlite3.connect(target / 'library.db')) as db:
            self.assertEqual(db.execute('SELECT status,path FROM downloads').fetchone(), ('interrupted', None))

    def test_legacy_database_without_continuous_recovery_tables_restores(self):
        with closing(sqlite3.connect(self.data / 'library.db')) as db:
            for table in ['sequence_recovery_receipts', 'sequence_recovery', 'sequence_receipts']:
                db.execute(f'DROP TABLE {table}')
            db.commit()
        self.create(); target = self.root / 'restored'; backup.restore(self.archive, target)
        self.assertEqual(backup.database_info(target / 'library.db')['core_id'], 'a09d5ea6-3895-4e8e-9a14-3739456484bc')


if __name__ == '__main__':
    unittest.main()
