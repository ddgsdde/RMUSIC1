PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
PRAGMA synchronous=FULL;
PRAGMA journal_size_limit=67108864;
PRAGMA wal_autocheckpoint=1000;
PRAGMA cache_size=-16384;
CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY, username TEXT NOT NULL UNIQUE COLLATE NOCASE,password TEXT NOT NULL,role TEXT NOT NULL,totp_secret TEXT,totp_pending TEXT,recovery TEXT NOT NULL DEFAULT '[]');
CREATE TABLE IF NOT EXISTS sessions(hash TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,expires INTEGER NOT NULL,name TEXT,scopes TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS sessions_expiry ON sessions(expires);
CREATE TABLE IF NOT EXISTS login_clients(session_hash TEXT PRIMARY KEY REFERENCES sessions(hash) ON DELETE CASCADE,client_id TEXT NOT NULL,device_name TEXT NOT NULL,device_type TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS login_clients_id ON login_clients(client_id);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS preferences(user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS recommendation_settings(user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,advanced INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO settings(key,value)
    SELECT 'recommendation_advanced', COALESCE((SELECT CASE WHEN r.advanced THEN 'true' ELSE 'false' END
        FROM recommendation_settings r JOIN users u ON u.id=r.user_id WHERE u.role='admin' ORDER BY u.rowid LIMIT 1),'false');
CREATE TABLE IF NOT EXISTS recommendation_feedback(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,PRIMARY KEY(user_id,track_id));
CREATE TABLE IF NOT EXISTS personal_daily_lists(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE, day TEXT NOT NULL, picks TEXT NOT NULL, PRIMARY KEY(user_id,day));
CREATE TABLE IF NOT EXISTS recommendation_queue_owner(zone_id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS external_accounts(user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,policy TEXT NOT NULL DEFAULT '{}',enabled INTEGER NOT NULL DEFAULT 1,created_at INTEGER NOT NULL,data_epoch INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS account_history(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,event_id TEXT NOT NULL,origin_core TEXT NOT NULL,origin_user TEXT NOT NULL,origin_id TEXT NOT NULL,started_at INTEGER NOT NULL,track_id TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)),PRIMARY KEY(user_id,event_id),UNIQUE(user_id,origin_core,origin_user,origin_id));
CREATE INDEX IF NOT EXISTS account_history_recent ON account_history(user_id,started_at DESC,event_id DESC);
CREATE INDEX IF NOT EXISTS account_history_track ON account_history(user_id,track_id,started_at DESC);
CREATE TABLE IF NOT EXISTS account_track_stats(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,track_id TEXT NOT NULL,play_count INTEGER NOT NULL DEFAULT 0,skip_count INTEGER NOT NULL DEFAULT 0,play_seconds REAL NOT NULL DEFAULT 0,last_played_at INTEGER,PRIMARY KEY(user_id,track_id));
CREATE INDEX IF NOT EXISTS account_recommendation_recent ON account_track_stats(user_id,last_played_at DESC);
CREATE TABLE IF NOT EXISTS account_listening_buckets(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,minute INTEGER NOT NULL,track_id TEXT NOT NULL,title TEXT NOT NULL,artist TEXT NOT NULL,cover_url TEXT,play_count INTEGER NOT NULL DEFAULT 0,skip_count INTEGER NOT NULL DEFAULT 0,play_seconds REAL NOT NULL DEFAULT 0,last_played_at INTEGER,PRIMARY KEY(user_id,minute,track_id));
CREATE TABLE IF NOT EXISTS account_session_owners(event_id TEXT PRIMARY KEY,user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,external INTEGER NOT NULL,data_epoch INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS external_resource_owners(kind TEXT NOT NULL,id TEXT NOT NULL,realm TEXT NOT NULL,user_id TEXT REFERENCES users(id) ON DELETE SET NULL,PRIMARY KEY(kind,id));
CREATE INDEX IF NOT EXISTS external_resource_user ON external_resource_owners(user_id,kind,id);
CREATE TABLE IF NOT EXISTS account_stat_origins(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,origin_core TEXT NOT NULL,origin_user TEXT NOT NULL,origin_id TEXT NOT NULL,minute INTEGER NOT NULL,track_id TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)),PRIMARY KEY(user_id,origin_core,origin_user,origin_id));
CREATE TABLE IF NOT EXISTS account_import_jobs(id TEXT PRIMARY KEY,admin_user TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,session_hash TEXT NOT NULL,state TEXT NOT NULL,manifest TEXT,bytes INTEGER NOT NULL DEFAULT 0,records INTEGER NOT NULL DEFAULT 0,applied INTEGER NOT NULL DEFAULT 0,target_user TEXT REFERENCES users(id) ON DELETE SET NULL,new_user INTEGER NOT NULL DEFAULT 0,error TEXT,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS account_import_rows(job_id TEXT NOT NULL REFERENCES account_import_jobs(id) ON DELETE CASCADE,sequence INTEGER NOT NULL,kind TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)),PRIMARY KEY(job_id,sequence));
CREATE TABLE IF NOT EXISTS account_origins(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,origin_core TEXT NOT NULL,origin_user TEXT NOT NULL,PRIMARY KEY(user_id,origin_core,origin_user));
CREATE TABLE IF NOT EXISTS roots(id TEXT PRIMARY KEY,path TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS tracks(id TEXT PRIMARY KEY,path TEXT UNIQUE,root_id TEXT,title TEXT NOT NULL,artist TEXT NOT NULL,album TEXT NOT NULL,album_artist TEXT NOT NULL,duration REAL NOT NULL DEFAULT 0,track_number INTEGER NOT NULL DEFAULT 0,disc_number INTEGER NOT NULL DEFAULT 0,source TEXT NOT NULL,format TEXT NOT NULL,sample_rate INTEGER NOT NULL DEFAULT 0,bit_depth INTEGER NOT NULL DEFAULT 0,channels INTEGER NOT NULL DEFAULT 0,bitrate INTEGER NOT NULL DEFAULT 0,cover_url TEXT,recording_id TEXT NOT NULL,available INTEGER NOT NULL DEFAULT 1,created_at INTEGER NOT NULL,mtime INTEGER NOT NULL DEFAULT 0,size INTEGER NOT NULL DEFAULT 0,scan_id TEXT,lyrics TEXT,provider_data TEXT NOT NULL DEFAULT '{}',album_id TEXT NOT NULL DEFAULT '',artist_id TEXT NOT NULL DEFAULT '');
CREATE INDEX IF NOT EXISTS tracks_title ON tracks(title,id);
CREATE INDEX IF NOT EXISTS tracks_available_title ON tracks(available,title,id);
CREATE INDEX IF NOT EXISTS tracks_root ON tracks(root_id);
CREATE INDEX IF NOT EXISTS tracks_source ON tracks(source,available);
CREATE INDEX IF NOT EXISTS tracks_album ON tracks(album_id,track_number);
CREATE INDEX IF NOT EXISTS tracks_artist ON tracks(artist_id,title);
CREATE INDEX IF NOT EXISTS tracks_recommend_artist ON tracks(source,available,artist_id,title);
CREATE INDEX IF NOT EXISTS tracks_recommend_album ON tracks(source,available,album_id,track_number);
CREATE INDEX IF NOT EXISTS tracks_created ON tracks(created_at DESC);
CREATE INDEX IF NOT EXISTS tracks_recording ON tracks(recording_id,source,available);
CREATE VIRTUAL TABLE IF NOT EXISTS track_search USING fts5(track_id UNINDEXED,tokens,tokenize='unicode61 remove_diacritics 2');
CREATE TABLE IF NOT EXISTS works(id TEXT PRIMARY KEY,title TEXT NOT NULL,metadata TEXT NOT NULL DEFAULT '{}');
CREATE TABLE IF NOT EXISTS recordings(id TEXT PRIMARY KEY,work_id TEXT REFERENCES works(id),title TEXT NOT NULL,version TEXT,provenance TEXT NOT NULL DEFAULT '{}');
CREATE TABLE IF NOT EXISTS releases(id TEXT PRIMARY KEY,title TEXT NOT NULL,artist TEXT NOT NULL,provenance TEXT NOT NULL DEFAULT '{}');
CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,kind TEXT NOT NULL,status TEXT NOT NULL,processed INTEGER NOT NULL DEFAULT 0,total INTEGER NOT NULL DEFAULT 0,error TEXT,created_at INTEGER NOT NULL,cursor TEXT);
CREATE TABLE IF NOT EXISTS zones(id TEXT PRIMARY KEY,data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sequence_receipts(zone_id TEXT NOT NULL REFERENCES zones(id) ON DELETE CASCADE,request_id TEXT NOT NULL,owner TEXT NOT NULL,payload_hash TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(zone_id,request_id));
CREATE TABLE IF NOT EXISTS sequence_recovery(zone_id TEXT PRIMARY KEY REFERENCES zones(id) ON DELETE CASCADE,data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sequence_output_holds(zone_id TEXT PRIMARY KEY REFERENCES zones(id) ON DELETE CASCADE,data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS sequence_recovery_receipts(zone_id TEXT NOT NULL REFERENCES zones(id) ON DELETE CASCADE,request_id TEXT NOT NULL,user_id TEXT NOT NULL,payload_hash TEXT NOT NULL,result_plan TEXT,created_at INTEGER NOT NULL,PRIMARY KEY(zone_id,request_id));
CREATE TABLE IF NOT EXISTS cache_entries(id TEXT PRIMARY KEY,track_id TEXT NOT NULL,format TEXT NOT NULL,size INTEGER NOT NULL DEFAULT 0,total_size INTEGER NOT NULL DEFAULT 0,ranges TEXT NOT NULL DEFAULT '[]',complete INTEGER NOT NULL DEFAULT 0,pinned INTEGER NOT NULL DEFAULT 0,last_access INTEGER NOT NULL,path TEXT NOT NULL,etag TEXT);
CREATE TABLE IF NOT EXISTS cache_exports(cache_id TEXT PRIMARY KEY REFERENCES cache_entries(id) ON DELETE CASCADE,path TEXT,size INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'pending',error TEXT,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS downloads(id TEXT PRIMARY KEY,track_id TEXT NOT NULL,status TEXT NOT NULL,size INTEGER NOT NULL DEFAULT 0,error TEXT,path TEXT,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS plugins(id TEXT PRIMARY KEY,enabled INTEGER NOT NULL DEFAULT 1,version TEXT NOT NULL,previous_version TEXT);
CREATE TABLE IF NOT EXISTS metadata_queue(album_id TEXT PRIMARY KEY,status TEXT NOT NULL DEFAULT 'pending',attempts INTEGER NOT NULL DEFAULT 0,next_try INTEGER NOT NULL DEFAULT 0,metadata TEXT);
CREATE TABLE IF NOT EXISTS artist_metadata(artist_id TEXT PRIMARY KEY,data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS playlists(id TEXT PRIMARY KEY,name TEXT NOT NULL,user_id TEXT NOT NULL,tracks TEXT NOT NULL DEFAULT '[]',smart_query TEXT);
CREATE INDEX IF NOT EXISTS playlists_user ON playlists(user_id,id);
CREATE TABLE IF NOT EXISTS provider_accounts(provider TEXT PRIMARY KEY,context TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS provider_playlist_directory(user_id TEXT NOT NULL,provider TEXT NOT NULL,account TEXT NOT NULL,remote_id TEXT NOT NULL,data TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(user_id,provider,account,remote_id));
CREATE TABLE IF NOT EXISTS playlist_bindings(local_id TEXT PRIMARY KEY REFERENCES playlists(id) ON DELETE CASCADE,user_id TEXT NOT NULL,provider TEXT NOT NULL,remote_id TEXT NOT NULL,account TEXT NOT NULL,capabilities TEXT NOT NULL DEFAULT '[]',baseline TEXT NOT NULL DEFAULT '{}',local_baseline TEXT NOT NULL DEFAULT '{}',revision TEXT NOT NULL DEFAULT '',local_version INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL DEFAULT 'pending',error TEXT,conflict TEXT,last_sync INTEGER NOT NULL DEFAULT 0,next_due INTEGER NOT NULL DEFAULT 0,auto_sync INTEGER NOT NULL DEFAULT 1,UNIQUE(user_id,provider,account,remote_id));
CREATE TABLE IF NOT EXISTS playlist_operations(id TEXT PRIMARY KEY,local_id TEXT NOT NULL REFERENCES playlist_bindings(local_id) ON DELETE CASCADE,version INTEGER NOT NULL,payload TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'pending',created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS play_sessions(id TEXT PRIMARY KEY,zone_id TEXT NOT NULL,track_id TEXT NOT NULL,title TEXT NOT NULL,artist TEXT NOT NULL,cover_url TEXT,duration REAL NOT NULL,started_at INTEGER NOT NULL,last_report_at INTEGER NOT NULL,last_position REAL NOT NULL,last_status TEXT NOT NULL,runtime_id TEXT NOT NULL,play_seconds REAL NOT NULL DEFAULT 0,counted INTEGER NOT NULL DEFAULT 0,finished_at INTEGER,finish_reason TEXT);
CREATE UNIQUE INDEX IF NOT EXISTS play_sessions_active_zone ON play_sessions(zone_id) WHERE finished_at IS NULL;
CREATE INDEX IF NOT EXISTS play_sessions_track ON play_sessions(track_id,started_at DESC);
CREATE INDEX IF NOT EXISTS play_sessions_recent ON play_sessions(started_at DESC);
CREATE TABLE IF NOT EXISTS offline_listens(user_id TEXT NOT NULL,client_id TEXT NOT NULL,session_id TEXT NOT NULL,payload TEXT NOT NULL,counted INTEGER NOT NULL,skipped INTEGER NOT NULL,PRIMARY KEY(user_id,client_id,session_id));
CREATE TABLE IF NOT EXISTS listen_groups(id TEXT PRIMARY KEY,track_id TEXT NOT NULL,title TEXT NOT NULL,artist TEXT NOT NULL,cover_url TEXT,duration REAL NOT NULL,started_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,seconds REAL NOT NULL DEFAULT 0,counted INTEGER NOT NULL DEFAULT 0,skipped INTEGER NOT NULL DEFAULT 0,skipped_at INTEGER,ended INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS listen_claims(playback_id TEXT PRIMARY KEY,group_id TEXT NOT NULL REFERENCES listen_groups(id),baseline REAL NOT NULL,timing_version INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS listen_claim_users(playback_id TEXT NOT NULL REFERENCES listen_claims(playback_id),user_id TEXT NOT NULL,PRIMARY KEY(playback_id,user_id));
CREATE TABLE IF NOT EXISTS listen_intervals(playback_id TEXT NOT NULL REFERENCES listen_claims(playback_id),start REAL NOT NULL,end REAL NOT NULL,PRIMARY KEY(playback_id,start));
CREATE TABLE IF NOT EXISTS listen_output_intervals(playback_id TEXT NOT NULL REFERENCES listen_claims(playback_id),start INTEGER NOT NULL CHECK(start>=0),end INTEGER NOT NULL CHECK(end>start),sample_rate INTEGER NOT NULL CHECK(sample_rate BETWEEN 8000 AND 768000),PRIMARY KEY(playback_id,start));
CREATE TABLE IF NOT EXISTS listen_output_cursors(playback_id TEXT PRIMARY KEY REFERENCES listen_claims(playback_id),frames INTEGER NOT NULL CHECK(frames>=0),sample_rate INTEGER NOT NULL,position REAL NOT NULL,origin_at INTEGER NOT NULL,base_frames INTEGER NOT NULL CHECK(base_frames>=0),runtime_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS group_output_totals(playback_id TEXT PRIMARY KEY REFERENCES listen_claims(playback_id),seconds REAL NOT NULL CHECK(seconds>=0 AND seconds<=86400));
CREATE TABLE IF NOT EXISTS listen_source_ends(playback_id TEXT PRIMARY KEY REFERENCES listen_claims(playback_id),position REAL NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS listen_session_sources(session_id TEXT PRIMARY KEY REFERENCES play_sessions(id) ON DELETE CASCADE,playback_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS track_stats(track_id TEXT PRIMARY KEY,play_count INTEGER NOT NULL DEFAULT 0,skip_count INTEGER NOT NULL DEFAULT 0,play_seconds REAL NOT NULL DEFAULT 0,last_played_at INTEGER);
CREATE INDEX IF NOT EXISTS track_stats_recent ON track_stats(last_played_at DESC);
CREATE TABLE IF NOT EXISTS daily_stats(day TEXT NOT NULL,track_id TEXT NOT NULL,title TEXT NOT NULL,artist TEXT NOT NULL,cover_url TEXT,play_count INTEGER NOT NULL DEFAULT 0,skip_count INTEGER NOT NULL DEFAULT 0,play_seconds REAL NOT NULL DEFAULT 0,last_played_at INTEGER,PRIMARY KEY(day,track_id));
CREATE INDEX IF NOT EXISTS daily_stats_track ON daily_stats(track_id,day);

CREATE TABLE IF NOT EXISTS sync_starts(id TEXT PRIMARY KEY,request TEXT NOT NULL,result TEXT NOT NULL,updated_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sync_controls(id TEXT PRIMARY KEY,group_id TEXT NOT NULL,request TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS output_groups(id TEXT PRIMARY KEY,data TEXT NOT NULL,deleted INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS devices(id TEXT PRIMARY KEY,name TEXT NOT NULL,custom_name TEXT,platform TEXT NOT NULL,outputs TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS output_settings(id TEXT PRIMARY KEY,config TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS cache_retired(path TEXT PRIMARY KEY,root TEXT NOT NULL,size INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sync_output_epochs(id TEXT PRIMARY KEY,group_id TEXT NOT NULL,listener_id TEXT NOT NULL,data TEXT NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS sync_output_epoch_group ON sync_output_epochs(group_id);
CREATE TABLE IF NOT EXISTS sync_output_final_receipts(epoch_id TEXT NOT NULL REFERENCES sync_output_epochs(id),endpoint_id TEXT NOT NULL,request_id TEXT NOT NULL UNIQUE,payload_hash TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(epoch_id,endpoint_id));
CREATE TABLE IF NOT EXISTS sync_group_holds(group_id TEXT NOT NULL,zone_id TEXT NOT NULL UNIQUE,anchor TEXT NOT NULL,PRIMARY KEY(group_id,zone_id));
CREATE TABLE IF NOT EXISTS sync_group_recoveries(request_id TEXT PRIMARY KEY,group_id TEXT NOT NULL,checkpoint_id TEXT NOT NULL,user_id TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sync_output_sources(epoch_id TEXT NOT NULL REFERENCES sync_output_epochs(id) ON DELETE CASCADE,endpoint_id TEXT NOT NULL,playback_id TEXT NOT NULL,queue_index INTEGER NOT NULL,account_id TEXT NOT NULL,grant TEXT NOT NULL,frames INTEGER NOT NULL DEFAULT 0,sample_rate INTEGER NOT NULL DEFAULT 0,position REAL NOT NULL,ended INTEGER NOT NULL DEFAULT 0,updated_at INTEGER NOT NULL,PRIMARY KEY(epoch_id,endpoint_id,playback_id),UNIQUE(epoch_id,endpoint_id,queue_index));

CREATE TABLE IF NOT EXISTS provider_directory_refresh(user_id TEXT NOT NULL,provider TEXT NOT NULL,account TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(user_id,provider,account));
CREATE TABLE IF NOT EXISTS provider_playlist_pages(id TEXT PRIMARY KEY,provider TEXT NOT NULL,account TEXT NOT NULL,data TEXT NOT NULL,updated_at INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS track_redirects(old_id TEXT PRIMARY KEY,new_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS cache_replacements(old_cache TEXT PRIMARY KEY,new_cache TEXT NOT NULL,old_track TEXT NOT NULL,new_track TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS cache_quality(cache_id TEXT PRIMARY KEY REFERENCES cache_entries(id) ON DELETE CASCADE,data TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS listening_metadata(track_id TEXT PRIMARY KEY,album TEXT NOT NULL,album_artist TEXT NOT NULL,source TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS maintenance_backups(id TEXT PRIMARY KEY,path TEXT NOT NULL,status TEXT NOT NULL,automatic INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,finished_at INTEGER,size INTEGER NOT NULL DEFAULT 0,error TEXT);
CREATE TABLE IF NOT EXISTS file_bin(id TEXT PRIMARY KEY,job_id TEXT NOT NULL,track_id TEXT NOT NULL,original_path TEXT NOT NULL,trash_path TEXT NOT NULL,storage TEXT NOT NULL,cache_id TEXT,cache_signature TEXT,status TEXT NOT NULL,size INTEGER NOT NULL DEFAULT 0,error TEXT,created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS file_bin_job ON file_bin(job_id,track_id);

-- Identity assertions never rewrite the playback/playlist track identity.
CREATE TABLE IF NOT EXISTS music_entities(id TEXT PRIMARY KEY,kind TEXT NOT NULL,namespace TEXT NOT NULL,external_id TEXT NOT NULL,title TEXT NOT NULL,UNIQUE(kind,namespace,external_id));
CREATE TABLE IF NOT EXISTS music_assertions(track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,entity_id TEXT NOT NULL REFERENCES music_entities(id),role TEXT NOT NULL,basis TEXT NOT NULL,position INTEGER NOT NULL DEFAULT 0,metadata TEXT NOT NULL DEFAULT '{}',PRIMARY KEY(track_id,entity_id,role,basis));
CREATE INDEX IF NOT EXISTS music_assertions_entity ON music_assertions(entity_id,track_id);
CREATE TABLE IF NOT EXISTS music_identity_revisions(track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,revision INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS music_identity_audit(id INTEGER PRIMARY KEY,track_id TEXT NOT NULL,revision INTEGER NOT NULL,user_id TEXT NOT NULL,before_data TEXT NOT NULL,after_data TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS music_identity_audit_track ON music_identity_audit(track_id,revision);
CREATE TABLE IF NOT EXISTS audio_fingerprints(track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,algorithm TEXT NOT NULL,source_stamp TEXT NOT NULL,data TEXT NOT NULL,created_at INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS backup_verifications(backup_id TEXT PRIMARY KEY REFERENCES maintenance_backups(id),verified_at INTEGER NOT NULL,ok INTEGER NOT NULL,message TEXT NOT NULL);

-- Minute buckets retain confirmed deltas for timezone-aware views without session guesses.
CREATE TABLE IF NOT EXISTS listening_buckets(minute INTEGER NOT NULL,track_id TEXT NOT NULL,title TEXT NOT NULL,artist TEXT NOT NULL,cover_url TEXT,play_count INTEGER NOT NULL DEFAULT 0,skip_count INTEGER NOT NULL DEFAULT 0,play_seconds REAL NOT NULL DEFAULT 0,last_played_at INTEGER,PRIMARY KEY(minute,track_id));

CREATE INDEX IF NOT EXISTS music_entities_lookup ON music_entities(kind,title COLLATE NOCASE,id);

CREATE INDEX IF NOT EXISTS jobs_kind_cursor ON jobs(kind,cursor);
CREATE INDEX IF NOT EXISTS jobs_kind_status ON jobs(kind,status);

CREATE TABLE IF NOT EXISTS playlist_import_receipts(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,request_id TEXT NOT NULL,payload_hash TEXT NOT NULL,playlist_id TEXT REFERENCES playlists(id) ON DELETE SET NULL,track_count INTEGER NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(user_id,request_id));

CREATE TABLE IF NOT EXISTS music_editions(track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,revision INTEGER NOT NULL,kind TEXT NOT NULL,label TEXT NOT NULL,notes TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS music_edition_audit(id INTEGER PRIMARY KEY,track_id TEXT NOT NULL,user_id TEXT NOT NULL,before_data TEXT NOT NULL,after_data TEXT NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS music_version_preferences(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,entity_id TEXT NOT NULL REFERENCES music_entities(id),track_id TEXT REFERENCES tracks(id) ON DELETE SET NULL,revision INTEGER NOT NULL,PRIMARY KEY(user_id,entity_id));

CREATE TABLE IF NOT EXISTS track_artists(track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,artist_id TEXT NOT NULL,name TEXT NOT NULL,position INTEGER NOT NULL,PRIMARY KEY(track_id,artist_id));
CREATE INDEX IF NOT EXISTS track_artists_artist ON track_artists(artist_id,track_id);

CREATE TABLE IF NOT EXISTS artwork_sources(id TEXT PRIMARY KEY,url TEXT NOT NULL,cover_id TEXT,fetched_at INTEGER);
CREATE TABLE IF NOT EXISTS catalog_profiles(kind TEXT NOT NULL,entity_id TEXT NOT NULL,automatic TEXT NOT NULL DEFAULT '{}',manual TEXT NOT NULL DEFAULT '{}',revision INTEGER NOT NULL DEFAULT 0,updated_at INTEGER NOT NULL DEFAULT (unixepoch()),PRIMARY KEY(kind,entity_id));
CREATE TABLE IF NOT EXISTS catalog_pages(cache_key TEXT NOT NULL,page INTEGER NOT NULL,data TEXT NOT NULL,updated_at INTEGER NOT NULL DEFAULT (unixepoch()),PRIMARY KEY(cache_key,page));

CREATE TABLE IF NOT EXISTS artwork_client_inputs(source_id TEXT PRIMARY KEY REFERENCES artwork_sources(id) ON DELETE CASCADE,cover_id TEXT NOT NULL,input_sha TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS artwork_source_cover ON artwork_sources(cover_id);
CREATE TABLE IF NOT EXISTS artwork_versions(cover_id TEXT NOT NULL,source_id TEXT NOT NULL REFERENCES artwork_sources(id),PRIMARY KEY(cover_id,source_id));
CREATE TABLE IF NOT EXISTS artwork_failures(source_id TEXT PRIMARY KEY REFERENCES artwork_sources(id),retry_at INTEGER NOT NULL,code TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS artwork_entities(kind TEXT NOT NULL,entity_id TEXT NOT NULL,source_id TEXT NOT NULL REFERENCES artwork_sources(id),PRIMARY KEY(kind,entity_id));
INSERT OR IGNORE INTO artwork_versions SELECT cover_id,id FROM artwork_sources WHERE cover_id IS NOT NULL;
INSERT OR IGNORE INTO artwork_entities
SELECT 'track',t.id,a.id
FROM tracks t JOIN artwork_sources a ON a.id=substr(t.cover_url,length('/api/v1/artwork/')+1)
WHERE t.cover_url GLOB '/api/v1/artwork/*';
INSERT OR IGNORE INTO artwork_entities
SELECT 'track',t.id,a.id
FROM tracks t JOIN artwork_sources a ON a.cover_id=substr(t.cover_url,length('/api/v1/covers/')+1)
WHERE t.cover_url GLOB '/api/v1/covers/*';

CREATE TABLE IF NOT EXISTS artwork_health(source_id TEXT PRIMARY KEY REFERENCES artwork_sources(id),state TEXT NOT NULL,error TEXT,checked_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS artwork_absences(source_id TEXT PRIMARY KEY REFERENCES artwork_sources(id),url TEXT NOT NULL,provider TEXT NOT NULL,kind TEXT NOT NULL,entity_id TEXT NOT NULL,name TEXT NOT NULL,checked_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS catalog_scrape_state(kind TEXT NOT NULL,entity_id TEXT NOT NULL,checked_at INTEGER NOT NULL DEFAULT 0,retry_at INTEGER NOT NULL DEFAULT 0,error TEXT,PRIMARY KEY(kind,entity_id));
CREATE TABLE IF NOT EXISTS provider_generations(provider TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS provider_daily_recommendations(provider TEXT PRIMARY KEY,generation INTEGER NOT NULL,version TEXT NOT NULL,day TEXT NOT NULL,data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS artwork_events(id INTEGER PRIMARY KEY,source_id TEXT NOT NULL,stage TEXT NOT NULL,code TEXT NOT NULL,status INTEGER NOT NULL,elapsed_ms INTEGER NOT NULL,created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS music_playback_policies(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,entity_id TEXT NOT NULL REFERENCES music_entities(id),revision INTEGER NOT NULL DEFAULT 1,data TEXT NOT NULL,PRIMARY KEY(user_id,entity_id));
CREATE TABLE IF NOT EXISTS catalog_aliases(kind TEXT NOT NULL,left_id TEXT NOT NULL,right_id TEXT NOT NULL,user_id TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(kind,left_id,right_id));
CREATE TABLE IF NOT EXISTS catalog_alias_audit(id INTEGER PRIMARY KEY,kind TEXT NOT NULL,left_id TEXT NOT NULL,right_id TEXT NOT NULL,linked INTEGER NOT NULL,user_id TEXT NOT NULL,created_at INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS catalog_alias_profile_baselines(kind TEXT NOT NULL,entity_id TEXT NOT NULL,data TEXT NOT NULL,PRIMARY KEY(kind,entity_id));

CREATE TABLE IF NOT EXISTS lyric_documents(id TEXT PRIMARY KEY,document TEXT NOT NULL CHECK(json_valid(document)),expires INTEGER NOT NULL,nonempty INTEGER NOT NULL,size INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS lyric_documents_expiry ON lyric_documents(expires,id);

CREATE INDEX IF NOT EXISTS play_sessions_heard_recent ON play_sessions(started_at DESC,id DESC) WHERE play_seconds>0;

-- Personal history is an indexed projection of confirmed listening sessions.
CREATE TRIGGER IF NOT EXISTS external_history_update AFTER UPDATE ON play_sessions
WHEN NEW.play_seconds>0 AND EXISTS(SELECT 1 FROM account_session_owners WHERE event_id=NEW.id AND external=1)
BEGIN
 INSERT INTO account_history(user_id,event_id,origin_core,origin_user,origin_id,started_at,track_id,data)
 SELECT o.user_id,NEW.id,json_extract(s.value,'$'),o.user_id,NEW.id,NEW.started_at,NEW.track_id,
 json_object('session_id',NEW.id,'track_id',NEW.track_id,'title',NEW.title,'artist',NEW.artist,'album',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN '' ELSE COALESCE(t.album,m.album,'') END,'album_artist',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN '' ELSE COALESCE(t.album_artist,m.album_artist,'') END,'cover_url',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN NEW.cover_url ELSE COALESCE(t.cover_url,NEW.cover_url) END,'source',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN 'unknown' ELSE COALESCE(m.source,t.source,'unknown') END,'started_at_ms',NEW.started_at,'finished_at_ms',NEW.finished_at,'finish_reason',NEW.finish_reason,'listened_seconds',NEW.play_seconds,'counted',json(CASE WHEN NEW.counted THEN 'true' ELSE 'false' END),'duration',NEW.duration,'imported',json('false'))
 FROM account_session_owners o JOIN external_accounts e ON e.user_id=o.user_id AND e.enabled=1 AND e.data_epoch=o.data_epoch JOIN settings s ON s.key='core_id'
 LEFT JOIN tracks t ON t.id=NEW.track_id LEFT JOIN listening_metadata m ON m.track_id=NEW.track_id
 WHERE o.event_id=NEW.id AND o.external=1
 ON CONFLICT(user_id,origin_core,origin_user,origin_id) DO UPDATE SET data=excluded.data;
END;

CREATE TRIGGER IF NOT EXISTS external_history_insert AFTER INSERT ON play_sessions
WHEN NEW.play_seconds>0 AND EXISTS(SELECT 1 FROM account_session_owners WHERE event_id=NEW.id AND external=1)
BEGIN
 INSERT INTO account_history(user_id,event_id,origin_core,origin_user,origin_id,started_at,track_id,data)
 SELECT o.user_id,NEW.id,json_extract(s.value,'$'),o.user_id,NEW.id,NEW.started_at,NEW.track_id,
 json_object('session_id',NEW.id,'track_id',NEW.track_id,'title',NEW.title,'artist',NEW.artist,'album',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN '' ELSE COALESCE(t.album,m.album,'') END,'album_artist',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN '' ELSE COALESCE(t.album_artist,m.album_artist,'') END,'cover_url',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN NEW.cover_url ELSE COALESCE(t.cover_url,NEW.cover_url) END,'source',CASE WHEN NEW.runtime_id='offline' AND NOT EXISTS(SELECT 1 FROM listen_groups issued WHERE issued.id=NEW.id) THEN 'unknown' ELSE COALESCE(m.source,t.source,'unknown') END,'started_at_ms',NEW.started_at,'finished_at_ms',NEW.finished_at,'finish_reason',NEW.finish_reason,'listened_seconds',NEW.play_seconds,'counted',json(CASE WHEN NEW.counted THEN 'true' ELSE 'false' END),'duration',NEW.duration,'imported',json('false'))
 FROM account_session_owners o JOIN external_accounts e ON e.user_id=o.user_id AND e.enabled=1 AND e.data_epoch=o.data_epoch JOIN settings s ON s.key='core_id'
 LEFT JOIN tracks t ON t.id=NEW.track_id LEFT JOIN listening_metadata m ON m.track_id=NEW.track_id
 WHERE o.event_id=NEW.id AND o.external=1
 ON CONFLICT(user_id,origin_core,origin_user,origin_id) DO UPDATE SET data=excluded.data;
END;

CREATE TABLE IF NOT EXISTS account_music_preferences(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,kind TEXT NOT NULL,entity_id TEXT NOT NULL,data TEXT NOT NULL CHECK(json_valid(data)),PRIMARY KEY(user_id,kind,entity_id));

CREATE INDEX IF NOT EXISTS cache_entries_path ON cache_entries(path);
CREATE INDEX IF NOT EXISTS cache_entries_complete_track ON cache_entries(track_id) WHERE complete=1;

CREATE TABLE IF NOT EXISTS account_listening_metadata(user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,track_id TEXT NOT NULL,album TEXT NOT NULL,album_artist TEXT NOT NULL,source TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(user_id,track_id));
