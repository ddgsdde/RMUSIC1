export type PlaylistFileFormat="json"|"m3u"|"m3u8"|"pls"|"csv";
export interface PlaylistImportPreview {items:{entry:{title:string;artist:string;album:string;location?:string};status:"matched"|"review"|"missing";matched:Pick<Track,"id"|"title"|"artist"|"album"|"duration"|"source"|"available">|null;candidates:Pick<Track,"id"|"title"|"artist"|"album"|"duration"|"source"|"available">[]}[];total:number;offset:number;limit:number;automatic_download:false}
export interface MusicIdentityAssertion {kind:"work"|"recording"|"release"|"artist";namespace:string;external_id:string;title:string;role:string;position?:number}
export interface MusicIdentity {track_id:string;revision:number;automatic_merge:false;entities:(MusicIdentityAssertion&{id:string;basis:string})[];relations:{from:string;to:string;relation:string;basis:string;track_id:string}[]}
export interface MusicIdentityPreview {before:MusicIdentity;after:MusicIdentity;preview_only:true;playback_ids_unchanged:true;affected_candidates:number;resolution:Record<string,{resolved:boolean;conflict:boolean;selected:(MusicIdentityAssertion&{id:string;basis:string})|null}>}
export interface VersionQualityEvidence {storage:"platform"|"local_file"|"core_cache";source:string;source_track_id:string;provider_declared?:{label?:string;verified?:boolean}|null;file_record:{format:string|null;sample_rate:number|null;bit_depth:number|null;channels:number|null;duration:number;verified:boolean};output_requires_live_report:true;labels_are_not_quality_certification:true}
export interface ArtworkCacheStatus {used_bytes:number;file_count:number;limit_bytes:number;total:number;offset:number;limit:number;protected_base_images:true;derived_count:number;metrics:{scope:"since_start";requests:number;not_modified:number;response_bytes:number;upstream_bytes:number;upstream_reads:number};items:{id:string;host:string|null;name:string;cover_id:string|null;state:string;error?:string|null;retry_at?:number|null}[];jobs:{id:string;kind:string;status:string;processed:number;total:number;phase:string;error?:string|null}[]}
export interface FingerprintEvidence {available:boolean;reason?:string;algorithm?:string;sampled_seconds?:number;automatic_merge:false;remote_lookup?:false;candidate_scan_limit?:number;candidate_scan_truncated?:boolean;offset?:number;next_offset?:number|null;items?:{track:Track;similarity:number;basis:string;recording_candidate:true;release_verified:false}[]}
export interface ExtensionMethod {method:string;title?:string;description?:string;kind:"metadata"|"statistics"|"automation";scope:"read"|"library"|"control"|"admin";effect:"none"|"proposal";input:Record<string,unknown>;output:Record<string,unknown>;context?:"library_summary_v1"}
export interface ExtensionResult<T=unknown> {plugin_id:string;method:string;version:1;effect:"none"|"proposal";host_actions_applied:false;output:T}
export interface GroupSyncPolicy {max_known_error_us:number;warmup_ms:number}
export interface CacheRepairJob {id:string;status:string;phase:string;processed:number;total:number;error?:string;options:{concurrency:number;checked?:number;skipped?:number;scan_complete?:boolean}}
export interface CacheRepairStatus {enabled:boolean;job:CacheRepairJob|null}
export interface Track {
  id: string;
  title: string;
  artist: string;
  album: string;
  album_artist?: string;
  duration: number;
  track_number?: number;
  disc_number?: number;
  source: string;
  format: string;
  sample_rate: number;
  bit_depth: number;
  channels: number;
  bitrate: number;
  cover_url?: string | null;
  recording_id: string;
  available: boolean;
  created_at?: number;
  integrated_lufs?: number | null;
  true_peak_dbfs?: number | null;
  provider_data?: Record<string, unknown>;
}
export interface SmartPlaylistQuery {
  storage?:string; format?:string; min_plays?:number; max_plays?:number;
  min_seconds?:number; max_seconds?:number; min_sample_rate?:number; min_bit_depth?:number;
  not_played_days?:number; favorites_only?:boolean; exclude_saved?:boolean;
  sort?:"title"|"recent"|"least_played"|"most_played"|"recent_added"; max_results?:number;
  q?: string;
  source?: string;
  album_id?: string;
  artist_id?: string;
}
export interface Playlist {
  cover_url?: string | null;
  id: string;
  name: string;
  track_ids: string[];
  alias_track_ids?:string[];
  smart_query?: SmartPlaylistQuery;
  truncated?: boolean;
  sync?: {provider:string;playlist_id:string;status:string;error?:string;last_sync:number;auto_sync:boolean;capabilities:string[];account_context:string;conflict?:{id?:string;reason:string;local:{name:string;track_ids:string[]};cloud:{name:string;track_ids:string[]}}};
}
export interface MusicEdition {revision:number;kind:string;label:string;notes:string}
export interface MusicVersionsPage {
  track_id:string;recording_entity_id:string|null;identity_resolved:boolean;
  items:{track:Track;edition:MusicEdition;same_recording:boolean;quality_evidence?:VersionQualityEvidence;identity?:MusicIdentity}[];
  total:number;offset:number;limit:number;candidate_scan_limited:boolean;
  preferred:{track_id:string|null;revision:number};other_recordings:Track[];
  other_page?:{offset:number;limit:number;total:number;next_offset:number|null};
  automatic_merge:false;edition:MusicEdition;
}
export interface MusicPlaybackPolicy {preferred_track_id:string|null;apply_to_new_playback:boolean;automatic_fallback:boolean;same_release:boolean;lossless_only:boolean}
export interface MusicPlaybackPolicyStatus {revision:number;policy:MusicPlaybackPolicy;identity_resolved:boolean;preferred_track:Track|null;applies_to:"new_explicit_play_commands";active_streams_unchanged:true}
export interface CatalogAliasItem {id:string;name:string;source:string;basis:string;direct?:boolean}
export interface CatalogAliasStatus {entity:CatalogAliasItem|null;items:CatalogAliasItem[];candidates:CatalogAliasItem[];revision:number;automatic_merge:false}
export interface CatalogAliasEdit {other_id:string;linked:boolean;expected_revision:number;preview_only?:boolean}
export interface CatalogAliasResult {revision:number;files_unchanged:true;preview_only?:boolean;left?:CatalogAliasItem;right?:CatalogAliasItem;members?:string[];automatic_merge?:false}
export interface ArtworkFailureEvent {source_id:string;stage:string;code:string;status:number;elapsed_ms:number;created_at:number}
export interface ArtworkFailureHistory {items:ArtworkFailureEvent[];limit:number;history_limit:number;contains_urls:false}
export interface MusicVersionQuery {offset?:number;other_offset?:number;limit?:number;q?:string;source?:string;sort?:"title"|"source"|"added";available?:boolean}
export interface SmartPlaylistPreview {
  items:Track[];total:number;truncated:boolean;offset:number;limit:number;statistics_scope:"shared_core";
}
export interface LlmConfiguration {
  revision:string;enabled:boolean;base_url:string;model:string;key_configured:boolean;
}
export interface LlmConfigurationEdit {
  expected_revision:string;enabled:boolean;base_url:string;model:string;key?:string;
}
export interface LlmRecommendations {
  items:Track[];reasons:Record<string,string>;model:string;ignored:number;
  source:"llm";sent_fields:string[];host_actions_applied:false;
}
export interface PluginDevelopmentCatalog {
  api_version:string;sdk_version:string;templates:{kind:string;title:string}[];
  commands:string[];limits:Record<string,number>;trust:string;contract_url:string;
}
export interface AudioInfo {
  track_id: string; source: string; format?: string; sample_rate?: number;
  bit_depth?: number; channels?: number; bitrate?: number; verified?: boolean;
  quality?: string; quality_label?: string; requested_quality?: string;
}
export interface RelatedCatalogPage {
  matched:boolean;source?:string;kind?:"album"|"artist";anchor?:Track;
  identity_basis?:"provider_id"|"exact_tags_duration_unique_candidate";
  collection?:{id:string;name:string};artists?:{mid:string;name:string}[];
  items:Track[];page?:number;page_size?:number;total?:number;next_page?:number|null;notice?:string;
}
export interface TrackBackground {
  matched: boolean; source: string; mid?: string; url?: string; fetched_at?: number;
  background?: { image_url?: string; description?: string; album_description?: string; genre?: string; language?: string; release_date?: string; label?: string; credits?: {role:string;names:string[]}[] };
  artists?: {name:string;mid?:string;image_url?:string;description?:string}[];
  warnings?: string[];
}
export interface TrackComments {
  supported:boolean;source?:string;kind?:"recommend"|"hot"|"new";page?:number;page_size?:number;
  items:{id:string;author:string;text:string;published_at:number;likes:number;reply_count:number;replies?:{id:string;author:string;text:string;published_at:number;reply_to?:string}[]}[];
  total?:number;has_more?:boolean;next_cursor?:string;fetched_at?:number;cached?:boolean;stale?:boolean;notice?:string;
}
export interface Album {
  id: string;
  title: string;
  artist: string;
  cover_url?: string;
  track_count: number;
}
export interface Artist {
  id: string;
  name: string;
  track_count: number;
  image_url?: string | null;
  description?: string | null;
}
export interface ListeningStats {
  all_time?:boolean;calendar_days?:number;chart_bucket_days?:number;
  timezone?:string;legacy_utc_seconds?:number;
  period_days: number;
  start_day: string;
  end_day: string;
  summary: {
    play_count: number;
    skip_count: number;
    listened_seconds: number;
    unique_tracks: number;
    unique_albums?:number;unique_artists?:number;active_days?:number;longest_streak?:number;last_played_at?:number|null;
  };
  days: {
    day: string;
    play_count: number;
    skip_count: number;
    listened_seconds: number;
  }[];
  tracks: {
    track_id: string;
    title: string;
    artist: string;
    cover_url: string | null;
    play_count: number;
    skip_count: number;
    listened_seconds: number;
    last_played_at: number | null;
  }[];
  artists: {
    artist: string;
    play_count: number;
    skip_count: number;
    listened_seconds: number;
    unique_tracks: number;
  }[];
  albums?:{album:string;artist:string;cover_url:string|null;play_count:number;skip_count:number;listened_seconds:number;unique_tracks:number;last_played_at:number|null}[];
  sources?:{source:string;listened_seconds:number;play_count:number}[];
  ranking_limit?:number;
}
export interface ListeningHistoryItem {session_id:string;track_id:string;title:string;artist:string;album:string;album_artist:string;cover_url:string|null;source:string;started_at_ms:number;finished_at_ms:number|null;finish_reason:string|null;listened_seconds:number;counted:boolean;duration:number}
export interface ListeningHistory {version:1;items:ListeningHistoryItem[];next_cursor:string|null;limit:number;time_basis:"confirmed_playback";day_timezone:"UTC";scope:"core"}
export interface OfflineListenOwner {core_id:string;user_id:string}
export interface OfflineListenDay {seconds:number;last_at:number}
export interface MeasuredListenSegment {start:number;end:number;last_at:number;output?:MeasuredOutputRange}
export interface MeasuredOutputRange {start:number;end:number;sample_rate:number}
export interface MeasuredOutputSegment extends MeasuredListenSegment {output:MeasuredOutputRange}
export interface MeasuredListen {version?:1|2;playback_id:string;listen_session_id:string;segments:MeasuredListenSegment[]}
export interface OfflineListenSnapshot {
  id:string;version:number;track_id:string;title:string;artist:string;duration:number;
  started_at:number;updated_at:number;position:number;days:Record<string,OfflineListenDay>;
  reason:"active"|"ended"|"stop"|"replaced"|"replay"|"interrupted";
  measured?:MeasuredListen;
}
export interface OfflineStatsBatch extends OfflineListenOwner {client_id:string;sessions:OfflineListenSnapshot[]}
export interface OfflineStatsAck {id:string;version:number}
export interface OfflineStatsResult {accepted:OfflineStatsAck[];rejected:(OfflineStatsAck&{code:"invalid_snapshot"|"conflicting_snapshot"})[];changed:boolean}
export interface SequencePrepareRequest {endpoint_id:string;track_id:string;playback_id:string;measured_version?:1|2}
export interface SequenceSlot {playback_id:string;listen_session_id?:string;track:Track;media_path:string;queue?:Track[];queue_index?:number}
export type SequencePrepared = {accepted:true;measured_version?:1|2;plan_id:string;listen_session_id?:string;from_playback_id:string;items:SequenceSlot[];has_more:boolean;refresh?:boolean;continuous_recovery?:boolean}|{accepted:false;zone:Zone};
export interface SequenceBoundary {playback_id:string;track_id:string;position:number;frames:number}
export interface DevicePlaybackReport extends SequenceBoundary {sample_rate:number;status?:Zone["status"];measured_version?:1|2}
export interface SequenceHandoffRequest {
  measured_version?:1|2;
  endpoint_id:string;request_id:string;plan_id:string;from_playback_id:string;sample_rate:number;
  ended:SequenceBoundary[];current:SequenceBoundary;
}
export interface SequenceHandoffResult {accepted:boolean;applied:boolean;request_id?:string;zone:Zone}
export interface SequenceRecoverRequest {
  measured_version?:1|2;
  endpoint_id:string;request_id:string;plan_id:string;sample_rate:number;
  heard:SequenceBoundary[];terminal:boolean;
}
export interface SequenceRecoverResult {measured_version?:1|2;accepted:boolean;applied:boolean;request_id?:string;plan_id?:string|null;finished?:boolean;waiting_for_play?:boolean;zone:Zone}
export type EqFilter = "peak" | "low_shelf" | "high_shelf" | "low_pass" | "high_pass" | "band_pass" | "notch" | "all_pass";
export type DspModule = "rate" | "stereo" | "loudness" | "gain" | "eq" | "fades" | "convolution";
export interface DspEqStage { bands:DspBand[]; bypass:boolean }
export interface DspBand {
  kind?: EqFilter;
  enabled: boolean;
  frequency: number;
  gain_db: number;
  q: number;
  slope_db_oct?: 12 | 24 | 36 | 48;
}
export interface DspConfig {
  enabled: boolean;
  preamp_db: number;
  balance: number;
  normalize: boolean;
  target_lufs: number;
  bands: DspBand[];
  modules?: DspModule[];
  extra_eq?: DspEqStage[];
  eq_bypass?: boolean;
  stereo?: {crossfeed:number;crossover_hz:number;mono:boolean;invert_left:boolean;invert_right:boolean}|null;
  preset_name?: string;
  sample_rate?: number | null;
  convolution?:ConvolutionConfig|null;
}
/** Older Cores may return a partial DSP object, including through zone events. */
export function normalizeDsp(config:Partial<DspConfig>|null|undefined):DspConfig {
  return {enabled:false,preamp_db:0,balance:0,normalize:false,target_lufs:-14,...config,bands:config?.bands??[]};
}
export interface ConvolutionConfig {response_ids:string[]}
export interface ImpulseAsset {
  id:string;name:string;bytes:number;sample_rate:number;channels:number;frames:number;channel_mask:number;created_at:number;
}
export interface ImpulseAssets {
  items:ImpulseAsset[];limits:{max_files:number;max_storage_bytes:number;max_upload_bytes:number};
}
export interface ImpulseDownload {asset:ImpulseAsset;api_path:string}
export interface ImpulseManifest {items:ImpulseDownload[];expires_at:number}
/** Native Core/bridge host mapping only. Nanosecond integers are read-only diagnostics in JS. */
export interface HostClockCalibration {
  version:1;
  core_clock_id:string;
  host_clock_id:string;
  host_minus_core_ns:number;
  network_rtt_ns:number;
  network_uncertainty_ns:number;
  observed_at_core_ns:number;
  valid_until_core_ns:number;
  sample_count:number;
  device_clock_mapped:false;
}
/** Driver timestamps are diagnostics; JS must not perform nanosecond scheduling with these numbers. */
export interface DeviceClockSample {
  version:1;epoch:number;source:"wasapi_ticks"|"alsa_frames"|"alsa_native_words"|"asio_frames"|"asio_dsd_bits"|"android_frames"|"core_audio_frames";
  timestamp_domain:"qpc"|"monotonic"|"driver_opaque"|"mach_absolute";counter:number;nominal_frequency_hz:number;
  device_time_ns:number;host_read_before_ns:number;host_read_after_ns:number;running:boolean;
  programme?:{frames:number;sample_rate:number};
}
/** Paired PCM programme position; no hardware precision or group ownership is asserted. */
export interface ProgrammeClockCalibration {
  version:1;zone_id:string;core_playback_id:string;track_id:string;position:number;source_frames:number;
  output_frame:number;sample_rate:number;device_epoch:number;device_counter:number;core_sample_ns:number|null;driver_accuracy_verified:false;
}
export interface DeviceClockCalibration {
  version:1;core_clock_id:string;host_clock_id:string;device_epoch:number;
  source:DeviceClockSample["source"];timestamp_domain:DeviceClockSample["timestamp_domain"];
  counter:number;nominal_frequency_hz:number;core_sample_ns:number|null;core_read_before_ns:number;core_read_after_ns:number;
  host_network_uncertainty_ns:number;driver_read_span_ns:number;valid_until_core_ns:number;running:boolean;
  device_clock_mapped:boolean;driver_accuracy_verified:false;
  programme_clock?:ProgrammeClockCalibration|null;
  rate_observation:{observed_frequency_hz:number;observed_drift_ppm:number;span_ns:number;sample_count:number;query_time_approximation:boolean;rate_sane:boolean}|null;
}
/** Core chooses integer clock deadlines; these JS nanosecond fields are read-only diagnostics. */
export interface SyncStartRequest {sync_policy?:GroupSyncPolicy;automatic_rate_adjustment?:boolean;pcm_rate_adjustment?:boolean;measured_version?:1|2;request_id:string;zone_ids:string[];track_id:string;position?:number;queue?:string[]}
/** Experimental manual target, ±500 ppm. playback_id is the original output epoch, not the current queue item. */
export interface GroupRateRequest {request_id:string;zone_id:string;playback_id:string;ppm:number}
export type GroupRecoveryState={available:true;checkpoint_id:string;queue_index:number;position:number}|{available:false;reason:string};
export interface GroupRecoveryRequest {request_id:string;checkpoint_id:string}
export type SyncControlRequest = {request_id:string} & ({action:"pause"|"resume"|"resync"}|{action:"seek";queue_index:number;position:number}|{action:"jump";from_queue_index:number;queue_index:number;position:number}|{action:"members";from_zone_ids:string[];zone_ids:string[]});
export interface GroupClockDiagnostics {
  version:1;state:"inactive"|"single_output"|"available"|"partial"|"unavailable";evaluated_at_core_ns:number;valid_for_ms:number;reference_zone_id:string|null;
  total_uncertainty_ns:null;driver_accuracy_verified:false;strict_sync:false;correction:"off"|"experimental";
  members:{zone_id:string;endpoint_id:string;available:boolean;reason:string|null;estimated_offset_ns:number|null;relative_drift_ppm:number|null;relative_programme_drift_ppm?:number|null;processing_projection_uncertainty_ns?:number|null;programme_clock_version?:1|2;source_per_output?:number;projection:"observed"|"nominal"|null;device_epoch?:number;valid_until_core_ns?:number;sample_age_ns?:number;driver_read_span_ns?:number;known_host_uncertainty_ns?:number}[];
}
export interface SyncStart {
  sync_policy?:GroupSyncPolicy|null;sync_monitor?:{version:1;state:"warming"|"within_known_bound"|"violated"|"inactive";reason?:string|null;max_known_error_us:number;observed_known_bound_ns?:number|null;fresh_samples:number;driver_accuracy_verified:false;acoustic_accuracy_verified:false;strict_sync:false;on_violation:"stop_group"}|null;
  buffer_recovery?:{count:number;active:boolean;changes_samples:false;resume_policy?:"earliest_confirmed"};
  measured_version?:1|2;
  rate_adjustment?:{enabled:true;mode:"manual"|"experimental_auto";automatic:boolean;last_request_id:string|null;reset_on_rearm:true;controller?:{state:"inactive"|"warming"|"tracking"|"held";reason:string|null;lease_ms:number;max_ppm:500;max_slew_ppm_per_second:20;driver_accuracy_verified:false;known_uncertainty_limit_ns?:number;realignments?:number}};
  id:string;track_id:string;title?:string|null;position:number;phase:"preparing"|"committed"|"playing"|"pausing"|"paused"|"seeking"|"ended"|"cancelled"|"failed"|"interrupted";
  error:string|null;created_at_core_ns:number;expires_at_core_ns:number;deadline_core_ns:number|null;may_have_started:boolean;statistics_scope:"group";strict_sync:false;settled?:boolean;
  queue?:{length:number;segment_start:number}|null;
  playhead?:{queue_index:number;position:number};listened_seconds?:number;clock_diagnostics?:GroupClockDiagnostics;
  transport?:{request_id:string;action:"pause"|"resume"|"resync"|"seek"|"jump"|"members";revision:number;state:"pending"|"applied"|"failed";error:string|null;incomplete_final_reports:boolean;target_zone_ids?:string[]|null}|null;
  members:{zone_id:string;endpoint_id:string;playback_id:string;requested_rate_ppm?:number;queue_index?:number;current_playback_id?:string|null;phase:"preparing"|"ready"|"playing"|"pausing"|"paused"|"ended"|"cancelled";deadline_host_ns:number;sample_rate:number;frames:number;position:number;final_report_received?:boolean;gate:{phase:string;expires_host_ns:number;deadline_host_ns:number;ready_host_ns:number;released_host_ns:number}|null}[];
}
export interface OutputGroup {id:string;name:string;zone_ids:string[];revision:number;updated_at:number}
export interface OutputGroupSave {name:string;zone_ids:string[];expected_revision:number}
/** Diagnostic snapshot; only Core may copy this into native scheduled-start tokens. */
export interface GroupPowerStamp {host_id:string;generation:number;wake_generation:number}
export interface Endpoint {
  device_volume?:{supported:boolean;value?:number};
  output_config?:OutputLimits&{device_id?:string;exclusive:boolean;buffer_ms:number;dsd_output?:"disabled"|"dop"|"native";android_mode?:"system"|"usb_bitperfect"|"compatibility"};
  group_power?:GroupPowerStamp|null;
  group_power_clock_id?:string|null;
  device_clock_calibration?:DeviceClockCalibration|null;
  clock_calibration?:HostClockCalibration|null;
  id: string;
  name: string;
  kind: "browser" | "wasapi" | "asio" | "alsa" | "airplay" | "hqplayer" | "coreaudio" | "android" | "group";
  group?:{id:string;zone_ids:string[];endpoint_ids:string[];phase:string;error?:string|null};
  online: boolean;
  capabilities: Record<string, unknown>;
  verified: boolean;
  playback?: {track_id?:string;core_playback_id?:string|null;status?:string;frames?:number;position?:number;reported_at?:number;signal_path?:{output?:string;sample_rate?:number;channels?:number;bit_depth?:number;exclusive?:boolean;sample_type?:string;mixer_sample_rate?:number;processing?:string[];bit_perfect_eligible?:boolean}};
}
export interface OutputLimits {max_sample_rate?:number|null;max_bit_depth?:number|null;max_dsd_rate?:number|null;channels?:number|null;resync_delay_ms?:number;volume_control?:"software"|"fixed"|"device";volume_max?:number;conversion_location?:"device"|"core"}
export interface Zone {
  id: string;
  name: string;
  endpoint_id: string;
  status: "playing" | "paused" | "stopped";
  buffering?: boolean;
  volume: number;
  muted: boolean;
  repeat: "off" | "all" | "one";
  shuffle: boolean;
  position: number;
  track: Track | null;
  playback_id?: string | null;
  queue: Track[];
  queue_revision?: number;
  queue_length?: number;
  queue_index: number;
  updated_at: number | string;
  dsp: DspConfig;
  autoplay?: AutoplayConfig;
  transitions?: {fade_in_ms:number;fade_out_ms:number};
}
export type RecommendationSource = string;
export interface AutoplayConfig {
  enabled: boolean;
  source: RecommendationSource;
  fallback_local: boolean;
}
export interface ProviderPlaylist {
  id: string;
  account_context?: string;
  name: string;
  track_count: number;
  cover_url: string | null;
  source: string;
  kind: "created" | "collected" | "liked";
  writable?: boolean;
  write_capabilities?: string[];
}
export interface ProviderPlaylistPage {
  items: ProviderPlaylist[];
  next_page: number | null;
}
export interface ProviderPlaylistTracks {
  items: Track[];
  total: number;
  next_page: number | null;
  playlist_id: string;
}
export interface RecommendationSection {id:string;kind?:"albums"|"tracks"|"playlists";title:string;source:string;items:Track[];playlists?:ProviderPlaylist[]}
export interface PlatformRecommendations {source:string;page:number;next_page:number|null;sections:RecommendationSection[]}
export interface Recommendations {
  sections?: RecommendationSection[];
  refreshing?: boolean;
  reasons?: Record<string,string>;
  algorithm?: "local"|"advanced";
  items: Track[];
  source: RecommendationSource;
  fallback: boolean;
  message?: string;
  seed_applied?: boolean;
}
export interface ProviderManifest {
  kind?:"source"|"extension";
  configuration_v1?:Record<string,unknown>;
  favorites_playlist_id?: string;
  id: string;
  name: string;
  version: string;
  api_version: string;
  enabled: boolean;
  status: string;
  capabilities: string[];
  quality_options?: {id:string;label:string}[];
  login_types?: string[];
  login_labels?: Record<string,string>;
  account_context?: string;
  error?: string;
}
export type ProviderLoginType = string;
export interface ProviderLoginSession {
  provider_name?:string;
  login_copy?:{title:string;scan:string;scanned:string;qrLabel:string};
  session_id: string;
  status: string;
  qr_data_url?: string | null;
  login_type?: ProviderLoginType | "mobile";
  error?: string;
}
export type ProviderLoginPoll = Pick<
  ProviderLoginSession,
  "status" | "qr_data_url" | "login_type" | "error"
>;
export interface LyricDocument {
  format: "lrc" | "plain" | "enhanced_lrc";
  text: string;
  translation?: string;
  romanization?:string;
  translations?: {id:string;language?:string;label:string;text:string}[];
  word_timing?: {time:number;text:string;words:{time:number;duration:number;text:string}[]}[];
  timing_source?:string;
  offset_ms: number;
  source: string;
}
export interface ThemeManifest {
  schema_version: 1;
  id: string;
  name: string;
  light: Record<string, string>;
  dark: Record<string, string>;
}
export interface PlaybackModule {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
  visible: boolean;
}
export interface PlaybackLayout {
  version: 1;
  breakpoint: "desktop" | "tablet" | "phone";
  modules: PlaybackModule[];
}
export interface CoreAddress {
  from_core?: boolean;
  address: string;
  certificate_sha256?: string;
}
export interface DiscoveredService {
  id:string; name:string; kind:"core"|"bridge"; addresses:string[]; core_id:string|null;
}
export interface ConnectionProfile {
  id: string;
  name: string;
  url: string;
  alternatives?: CoreAddress[];
}
export interface User {
  id: string;
  username: string;
  role: string;
  totp_enabled?: boolean;
  policy?: ExternalSourcePolicy;
}
export interface ExternalSourcePolicy {
  providers: string[];
  local_cache: boolean;
  local_library: boolean;
  bandwidth_bytes_per_second: number;
}
export interface ExternalAccount extends User {
  role: "external";
  policy: ExternalSourcePolicy;
  enabled: boolean;
  created_at: number;
}
export interface AccountImportJob {
  id: string;
  state: string;
  records?: number;
  applied?: number;
  bytes?: number;
  error?: string;
  target_user?: string;
  counts?: Record<string, number>;
  manifest?: {username: string; origin_core: string; origin_user: string};
  merge_candidates?: {id: string; username: string}[];
}
export interface AuthResult {
  token: string;
  user: User;
}
export interface Page<T> {
  items: T[];
  total: number;
}
export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}
export interface ConnectionMetrics {
  mode:string; underlay?:string; address:string; latency_ms:number;
  end_to_end?:boolean; down_mbps?:number;
}
export class RMusicClient {
  private zoneScope?: {base:string;token:string;queues:Map<string,{revision:number;epoch:string;items:Track[]}>;pending?:Promise<unknown>};
  artworkRevision=0;
  constructor(
    public baseUrl = "",
    public token = "",
  ) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
  }
  url(path: string) {
    return `${this.baseUrl}${path}`;
  }
  fetchResource(url:string|URL, options:RequestInit={}):Promise<Response> { return fetch(url,options); }
  private async response(path: string, options: RequestInit = {}): Promise<Response> {
    const headers = new Headers(options.headers);
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    if (options.body && typeof options.body === "string")
      headers.set("Content-Type", "application/json");
    const response = await this.fetchResource(this.url(`/api/v1${path}`), {
      ...options,
      headers,
    });
    if (!response.ok) {
      let body;
      try {
        body = await response.json();
      } catch {
        body = null;
      }
      throw new ApiError(
        response.status,
        body?.error?.code ?? "http_error",
        body?.error?.message ?? `请求失败 (${response.status})`,
      );
    }
    return response;
  }
  visualization(id:string,after?:number,signal?:AbortSignal){return this.request<{version:1;supported:boolean;playback_id:string|null;track_id?:string;position?:number;frames:{playback_id:string;position:number;bands:number[];energy:number;peak:number}[]}>(`/zones/${encodeURIComponent(id)}/visualization`,{method:"POST",body:JSON.stringify({after}),signal});}
  async connectionMetrics(speed:boolean,signal?:AbortSignal):Promise<ConnectionMetrics> {
    const opts:RequestInit={cache:"no-store",redirect:"error",signal};
    const elapsed:number[]=[];let ingress="";let onLink=false;
    for(let n=0;n<3;n++){
      const start=performance.now();const reply=await this.request<{ingress:string;on_link?:boolean}>("/connection-probe",opts);
      elapsed.push(performance.now()-start);ingress=reply.ingress;onLink=reply.on_link===true;
    }
    const base=new URL(this.baseUrl||location.origin,location.origin);
    const local=["localhost","127.0.0.1","[::1]"].includes(base.hostname);
    const h=base.hostname.toLowerCase();
    const privateHost=/^(10\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.)/.test(h)||/^\[f[cd][0-9a-f]{2}:/.test(h)||h.endsWith(".local");
    const mode=ingress==="reverse_proxy"?ingress:local?"local":privateHost?(onLink?"lan":"private"):base.protocol==="https:"?"https":"direct";
    const result:ConnectionMetrics={mode,address:base.toString().replace(/\/$/,""),latency_ms:elapsed.sort((a,b)=>a-b)[1]};
    if(speed){
      const size=1024*1024;const start=performance.now();const response=await this.response("/connection-probe/payload",opts);
      const reader=response.body?.getReader();if(!reader)throw new Error("浏览器无法读取测速数据");
      let offset=0;
      try{for(;;){const item=await reader.read();if(item.done)break;if(offset+item.value.length>size){await reader.cancel();throw new Error("测速响应过大");}offset+=item.value.length;}}finally{reader.releaseLock();}
      if(offset!==size)throw new Error("测速下载未完成");
      result.down_mbps=size*0.008/Math.max(0.01,performance.now()-start);
    }
    return result;
  }
  async request<T>(path: string, options: RequestInit = {}): Promise<T> {
    if(path==="/zones" && (!options.method || options.method==="GET")) {
      let scope=this.zoneScope;
      if(!scope || scope.base!==this.baseUrl || scope.token!==this.token) {
        scope={base:this.baseUrl,token:this.token,queues:new Map()};this.zoneScope=scope;
      }
      if(options.signal)return this.zoneSnapshot(scope,options) as Promise<T>;
      if(!scope.pending) {
        const current=scope;
        current.pending=this.zoneSnapshot(current,options).finally(()=>{current.pending=undefined;});
      }
      return scope.pending as Promise<T>;
    }
    if(path==="/auth/logout")this.zoneScope=undefined;
    if(["POST","PUT"].includes(options.method||"") && /^\/zones\/[^/]+\/queue(?:\/reorder)?$/.test(path.split("?")[0])) {
      const target=new URLSearchParams(path.split("?")[1]).has("view")?path:path+(path.includes("?")?"&":"?")+"view=state";
      const result=await this.rawRequest<Zone>(target,options);
      if(typeof result.queue_length!=="number")return result as T;
      const page=await this.request<Page<Zone>>("/zones",{signal:options.signal||new AbortController().signal});
      const zone=page.items.find(zone=>zone.id===result.id);
      if(!zone)throw new Error("音区已移除，请重新选择播放设备");
      return zone as T;
    }

    if((path.split("?")[0]==="/library/tracks"||/^\/playlists\/[^/]+\/tracks$/.test(path.split("?")[0])) && (!options.method||options.method==="GET") && !new URLSearchParams(path.split("?")[1]).has("view"))path+=(path.includes("?")?"&":"?")+"view=compact";
    return this.rawRequest<T>(path,options);
  }
  private async rawRequest<T>(path:string,options:RequestInit={}):Promise<T> {
    const response=await this.response(path,options);
    if(path.includes('refresh=true')||(path.startsWith('/catalog/')&&options.method&&options.method!=='GET'))this.artworkRevision++;
    if (response.status === 204) return undefined as T;
    return response.json();
  }
  private async zoneSnapshot(scope:NonNullable<RMusicClient["zoneScope"]>,options:RequestInit) {
    type Snapshot=Page<Zone>&{queue_view?:string;queue_epoch?:string};
    const current=()=>{options.signal?.throwIfAborted();if(this.zoneScope!==scope || this.baseUrl!==scope.base || this.token!==scope.token)throw new DOMException("Core 会话已切换","AbortError");};
    for(let attempt=0;attempt<2;attempt++) {
      current();
      const state=await this.rawRequest<Snapshot>("/zones?view=state",options);
      current();
      if(state.queue_view!=="versioned")return state;
      const epoch=state.queue_epoch||"";
      let next=0;
      try {
        await Promise.all(Array.from({length:Math.min(3,state.items.length)},async()=>{
          while(next<state.items.length) {
            const zone=state.items[next++],revision=zone.queue_revision!,total=zone.queue_length!;
            let cached=scope.queues.get(zone.id);
            if(!cached || cached.epoch!==epoch || cached.revision!==revision || cached.items.length!==total) {
              const items:Track[]=[];
              while(items.length<total) {
                current();
                const page=await this.rawRequest<{revision:number;epoch?:string;offset:number;total:number;items:Track[]}>(`/zones/${encodeURIComponent(zone.id)}/queue?epoch=${encodeURIComponent(epoch)}&revision=${revision}&offset=${items.length}&limit=500`,options);
                current();
                if((page.epoch||"")!==epoch || page.revision!==revision || page.total!==total || page.offset!==items.length || !page.items.length || items.length+page.items.length>total)throw new Error("Core 队列分页发生变化");
                items.push(...page.items);
              }
              cached={revision,epoch,items};scope.queues.set(zone.id,cached);
            }
            zone.queue=[...cached.items];
          }
        }));
      } catch(error) {
        if(attempt===0 && error instanceof ApiError && error.status===409)continue;
        throw error;
      }
      const live=new Set(state.items.map(zone=>zone.id));
      current();
      for(const id of scope.queues.keys())if(!live.has(id))scope.queues.delete(id);
      return state;
    }
    throw new Error("队列正在更新，请稍后重试");
  }
  uploadAccountArchive(job: string, file: Blob, signal?: AbortSignal) {
    return this.request<AccountImportJob>(`/auth/account-imports/${encodeURIComponent(job)}/file`, {
      method: "PUT", headers: {"Content-Type": "application/x-ndjson"}, body: file, signal,
    });
  }
  previewPlaylistImport(body:{format:PlaylistFileFormat;content:string;offset?:number;limit?:number},signal?:AbortSignal){return this.request<PlaylistImportPreview>("/playlists/import/preview",{method:"POST",body:JSON.stringify(body),signal});}
  importPlaylist(body:{name:string;track_ids:string[];request_id?:string},signal?:AbortSignal){return this.request<{id:string;count:number}>("/playlists/import",{method:"POST",body:JSON.stringify(body),signal});}
  trackIdentity(id:string,signal?:AbortSignal){return this.request<MusicIdentity>(`/library/tracks/${encodeURIComponent(id)}/identity`,{signal});}
  identityHistory(id:string,offset=0,limit=30,signal?:AbortSignal){return this.request<{items:{revision:number;before:MusicIdentity;after:MusicIdentity;created_at:number}[];total:number}>(`/library/tracks/${encodeURIComponent(id)}/identity/history?offset=${offset}&limit=${limit}`,{signal});}
  searchMusicEntities(q:string,kind:string,signal?:AbortSignal,offset=0){return this.request<{items:(MusicIdentityAssertion&{id:string})[];total:number;offset:number}>(`/music/entities?${new URLSearchParams({q,kind,offset:String(offset)})}`,{signal});}
  musicEntity(id:string,offset=0,limit=30,signal?:AbortSignal){return this.request<{entity:{id:string;kind:string;namespace:string;external_id:string;title:string};items:Track[];total:number;offset:number;limit:number}>(`/music/entities/${encodeURIComponent(id)}?offset=${offset}&limit=${limit}`,{signal});}
  indexMusicIdentity(signal?:AbortSignal){return this.request<{id:string;status:string}>("/music/index",{method:"POST",signal});}
  editTrackIdentity(id:string,body:{expected_revision:number;assertions:MusicIdentityAssertion[]},signal?:AbortSignal){return this.request<MusicIdentity>(`/library/tracks/${encodeURIComponent(id)}/identity`,{method:"PUT",body:JSON.stringify(body),signal});}
  previewTrackIdentity(id:string,body:{expected_revision:number;assertions:MusicIdentityAssertion[]},signal?:AbortSignal){return this.request<MusicIdentityPreview>(`/library/tracks/${encodeURIComponent(id)}/identity/preview`,{method:"POST",body:JSON.stringify(body),signal});}
  restoreTrackIdentity(id:string,expected_revision:number,revision:number,signal?:AbortSignal){return this.request<MusicIdentity>(`/library/tracks/${encodeURIComponent(id)}/identity/restore`,{method:"POST",body:JSON.stringify({expected_revision,revision}),signal});}
  artworkCache(offset=0,failed=false,signal?:AbortSignal){return this.request<ArtworkCacheStatus>(`/artwork-cache?offset=${offset}&limit=50&failed=${failed}`,{signal});}
  inspectArtwork(signal?:AbortSignal){return this.request<{id:string;status:string}>("/artwork-cache/check",{method:"POST",signal});}
  refreshArtwork(ids:string[],signal?:AbortSignal){return this.request<{id:string;status:string}>("/artwork-cache/refresh",{method:"POST",body:JSON.stringify({ids}),signal});}
  fingerprint(id:string,signal?:AbortSignal,offset=0){return this.request<FingerprintEvidence>(`/library/tracks/${encodeURIComponent(id)}/fingerprint?offset=${offset}`,{signal});}
  analyzeFingerprint(id:string){return this.request<{id:string;status:string}>(`/library/tracks/${encodeURIComponent(id)}/fingerprint`,{method:"POST"});}
  extensions(signal?:AbortSignal){return this.request<{items:{id:string;name:string;version:string;extension_api:"1";extensions:ExtensionMethod[];enabled:boolean}[]}>("/extensions",{signal});}
  invokeExtension<T=unknown>(id:string,method:string,input:unknown,signal?:AbortSignal){return this.request<ExtensionResult<T>>(`/extensions/${encodeURIComponent(id)}/invoke/${encodeURIComponent(method)}`,{method:"POST",body:JSON.stringify(input),signal});}
  pluginConfiguration(id:string,signal?:AbortSignal){return this.request<{schema:Record<string,unknown>;values:Record<string,unknown>;revision:string}>(`/plugins/${encodeURIComponent(id)}/configuration`,{signal});}
  configurePlugin(id:string,body:{expected_revision:string;values:Record<string,unknown>},signal?:AbortSignal){return this.request<{saved:true;revision:string}>(`/plugins/${encodeURIComponent(id)}/configuration`,{method:"PUT",body:JSON.stringify(body),signal});}
  startGroup(body:SyncStartRequest,signal?:AbortSignal){return this.request<SyncStart>("/sync/starts",{method:"POST",body:JSON.stringify(body),signal});}
  listOutputGroups(signal?:AbortSignal){return this.request<{items:OutputGroup[]}>("/output-groups",{signal});}
  saveOutputGroup(id:string,body:OutputGroupSave,signal?:AbortSignal){return this.request<OutputGroup>(`/output-groups/${encodeURIComponent(id)}`,{method:"PUT",body:JSON.stringify(body),signal});}
  deleteOutputGroup(id:string,revision:number,signal?:AbortSignal){return this.request<void>(`/output-groups/${encodeURIComponent(id)}?expected_revision=${revision}`,{method:"DELETE",signal});}
  listGroupStarts(options:{active?:boolean;recoverable?:boolean;limit?:number;offset?:number}={},signal?:AbortSignal){const q=new URLSearchParams();for(const [key,value]of Object.entries(options)){if(value!==undefined)q.set(key,String(value));}return this.request<{items:SyncStart[];total:number}>(`/sync/starts${q.size?`?${q}`:""}`,{signal});}
  getGroupStart(id:string,signal?:AbortSignal){return this.request<SyncStart>(`/sync/starts/${encodeURIComponent(id)}`,{signal});}
  controlGroupStart(id:string,body:SyncControlRequest,signal?:AbortSignal){return this.request<SyncStart>(`/sync/starts/${encodeURIComponent(id)}/control`,{method:"POST",body:JSON.stringify(body),signal});}
  setGroupRate(id:string,body:GroupRateRequest,signal?:AbortSignal){return this.request<SyncStart>(`/sync/starts/${encodeURIComponent(id)}/rate`,{method:"POST",body:JSON.stringify(body),signal});}
  cancelGroupStart(id:string,signal?:AbortSignal){return this.request<SyncStart>(`/sync/starts/${encodeURIComponent(id)}/cancel`,{method:"POST",signal});}
  getGroupRecovery(id:string,signal?:AbortSignal){return this.request<GroupRecoveryState>(`/sync/starts/${encodeURIComponent(id)}/recovery`,{signal});}
  recoverGroupStart(id:string,body:GroupRecoveryRequest,signal?:AbortSignal){return this.request<SyncStart>(`/sync/starts/${encodeURIComponent(id)}/recovery`,{method:"POST",body:JSON.stringify(body),signal});}
  listImpulses(signal?:AbortSignal){return this.request<ImpulseAssets>("/dsp/impulses",{signal});}
  importImpulse(bytes:Blob|ArrayBuffer,name?:string,signal?:AbortSignal){
    return this.request<ImpulseAsset>(`/dsp/impulses${name===undefined?"":`?name=${encodeURIComponent(name)}`}`,{method:"POST",headers:{"Content-Type":"audio/wav"},body:bytes,signal});
  }
  getImpulse(id:string,signal?:AbortSignal){return this.request<ImpulseAsset>(`/dsp/impulses/${encodeURIComponent(id)}`,{signal});}
  deleteImpulse(id:string,signal?:AbortSignal){return this.request<void>(`/dsp/impulses/${encodeURIComponent(id)}`,{method:"DELETE",signal});}
  async impulseFile(id:string,signal?:AbortSignal){return (await this.response(`/dsp/impulses/${encodeURIComponent(id)}/file`,{signal})).arrayBuffer();}
  resolveImpulses(zoneId:string,responseIds:string[],signal?:AbortSignal){return this.request<ImpulseManifest>(`/zones/${encodeURIComponent(zoneId)}/dsp/impulses`,{method:"POST",body:JSON.stringify({response_ids:responseIds}),signal});}
  get<T>(path: string) {
    return this.request<T>(path);
  }
  syncOfflineStats(body:OfflineStatsBatch,signal?:AbortSignal){
    return this.request<OfflineStatsResult>("/stats/offline",{method:"POST",body:JSON.stringify(body),signal});
  }
  prepareSequence(zoneId:string,body:SequencePrepareRequest,signal?:AbortSignal){
    return this.request<SequencePrepared>(`/zones/${encodeURIComponent(zoneId)}/prepare`,{method:"POST",body:JSON.stringify(body),signal});
  }
  disconnectBrowserEndpoint(endpointId:string,signal?:AbortSignal){
    return this.request<void>(`/endpoints/browser/${encodeURIComponent(endpointId)}/disconnect`,{method:"POST",signal});
  }
  handoffSequence(zoneId:string,body:SequenceHandoffRequest,signal?:AbortSignal){
    return this.request<SequenceHandoffResult>(`/zones/${encodeURIComponent(zoneId)}/handoff`,{method:"POST",body:JSON.stringify(body),signal});
  }
  recoverSequence(zoneId:string,body:SequenceRecoverRequest,signal?:AbortSignal){
    return this.request<SequenceRecoverResult>(`/zones/${encodeURIComponent(zoneId)}/recover`,{method:"POST",body:JSON.stringify(body),signal});
  }
  reportRecoveryUnavailable(zoneId:string,endpointId:string,signal?:AbortSignal){
    return this.request<{applied:boolean;zone:Zone}>(`/zones/${encodeURIComponent(zoneId)}/recover/unavailable`,{method:"POST",body:JSON.stringify({endpoint_id:endpointId}),signal});
  }
  post<T>(path: string, body?: unknown) {
    return this.request<T>(path, {
      method: "POST",
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  }
  put<T>(path: string, body: unknown) {
    return this.request<T>(path, { method: "PUT", body: JSON.stringify(body) });
  }
  patch<T>(path: string, body: unknown) {
    return this.request<T>(path, {
      method: "PATCH",
      body: JSON.stringify(body),
    });
  }
  delete<T = void>(path: string) {
    return this.request<T>(path, { method: "DELETE" });
  }
  providerPlaylists(
    providerId: string,
    collection: "all" | "created" | "collected" = "all",
    page = 1,
  ) {
    return this.get<ProviderPlaylistPage>(
      `/providers/${encodeURIComponent(providerId)}/playlists?collection=${collection}&page=${page}`,
    );
  }
  providerPlaylistTracks(
    providerId: string,
    playlistId: string,
    page = 1,
    signal?: AbortSignal,
  ) {
    return this.request<ProviderPlaylistTracks>(
      `/providers/${encodeURIComponent(providerId)}/playlists/${encodeURIComponent(playlistId)}/tracks?page=${page}`,
      { signal },
    );
  }
  trackVersions(id:string,offset=0,signal?:AbortSignal) {
    return this.request<MusicVersionsPage>(`/library/tracks/${encodeURIComponent(id)}/versions?offset=${offset}`,{signal});
  }
  queryTrackVersions(id:string,query:MusicVersionQuery={},signal?:AbortSignal) {
    const params=new URLSearchParams();for(const [k,v] of Object.entries(query))if(v!==undefined)params.set(k,String(v));
    return this.request<MusicVersionsPage>(`/library/tracks/${encodeURIComponent(id)}/versions?${params}`,{signal});
  }
  musicPlaybackPolicy(id:string,signal?:AbortSignal) {return this.request<MusicPlaybackPolicyStatus>(`/library/tracks/${encodeURIComponent(id)}/versions/policy`,{signal});}
  setMusicPlaybackPolicy(id:string,input:{expected_revision:number;policy:MusicPlaybackPolicy},signal?:AbortSignal) {return this.request<{revision:number;policy:MusicPlaybackPolicy;active_streams_unchanged:true}>(`/library/tracks/${encodeURIComponent(id)}/versions/policy`,{method:"PUT",body:JSON.stringify(input),signal});}
  catalogAliases(kind:"artist"|"album",id:string,q="",signal?:AbortSignal) {return this.request<CatalogAliasStatus>(`/library/catalog/${kind}/${encodeURIComponent(id)}/links?q=${encodeURIComponent(q)}`,{signal});}
  editCatalogAlias(kind:"artist"|"album",id:string,input:CatalogAliasEdit,signal?:AbortSignal) {return this.request<CatalogAliasResult>(`/library/catalog/${kind}/${encodeURIComponent(id)}/links`,{method:"PUT",body:JSON.stringify(input),signal});}
  artworkFailureHistory(source?:string,signal?:AbortSignal) {return this.request<ArtworkFailureHistory>(`/artwork-cache/events${source?`?source=${encodeURIComponent(source)}`:""}`,{signal});}
  preferMusicVersion(id:string,input:{expected_revision:number;track_id:string|null},signal?:AbortSignal) {
    return this.request<{revision:number;track_id:string|null;applies_to:string;playback_ids_unchanged:true}>(`/library/tracks/${encodeURIComponent(id)}/versions`,{method:"PUT",body:JSON.stringify(input),signal});
  }
  editMusicEdition(id:string,input:Omit<MusicEdition,"revision">&{expected_revision:number},signal?:AbortSignal) {
    return this.request<MusicEdition>(`/library/tracks/${encodeURIComponent(id)}/edition`,{method:"PUT",body:JSON.stringify(input),signal});
  }
  smartPlaylistPreview(rule:SmartPlaylistQuery,offset=0,signal?:AbortSignal) {
    return this.request<SmartPlaylistPreview>("/smart-playlists/preview",{method:"POST",body:JSON.stringify({rule,offset}),signal});
  }
  llmSettings(signal?:AbortSignal) {return this.request<LlmConfiguration>("/llm/settings",{signal});}
  configureLlm(input:LlmConfigurationEdit,signal?:AbortSignal) {
    return this.request<LlmConfiguration>("/llm/settings",{method:"PUT",body:JSON.stringify(input),signal});
  }
  llmModels(overrides:{base_url?:string;key?:string}={},signal?:AbortSignal) {
    return this.request<{items:string[]}>("/llm/models",{method:"POST",body:JSON.stringify(overrides),signal});
  }
  llmRecommendations(input:{track_ids:string[];preference?:string},signal?:AbortSignal) {
    return this.request<LlmRecommendations>("/llm/recommend",{method:"POST",body:JSON.stringify(input),signal});
  }
  pluginDevelopment(signal?:AbortSignal) {return this.request<PluginDevelopmentCatalog>("/plugins/development",{signal});}
  checkPluginContract(id:string,signal?:AbortSignal) {
    return this.request<{id:string;passed:boolean;checks:string[];uses_installed_credentials:false}>(`/plugins/${encodeURIComponent(id)}/contract-check`,{method:"POST",body:"{}",signal});
  }
  recommendations(options: {
    refresh?: boolean;
    source: RecommendationSource;
    seed_track_id?: string;
    limit?: number;
    fallback_local?: boolean;
  }) {
    const query = new URLSearchParams({
      source: options.source,
      limit: String(options.limit ?? 20),
      fallback_local: String(options.fallback_local ?? true),
    });
    if (options.seed_track_id)
      query.set("seed_track_id", options.seed_track_id);
    if (options.refresh) query.set("refresh", "true");
    return this.get<Recommendations>(`/recommendations?${query}`);
  }
  async mediaUrl(trackId: string, signal?: AbortSignal) {
    const result = await this.request<{ url: string; api_path?: string; expires_at: number }>(
      `/library/tracks/${encodeURIComponent(trackId)}/ticket`,
      {signal},
    );
    const path = result.api_path ? this.url(`/${result.api_path}`) : result.url;
    return new URL(path, this.baseUrl || window.location.href).href;
  }
  async pcmUrl(trackId:string,rate:number,start=0,signal?:AbortSignal){
    if(!Number.isInteger(rate)||rate<8000||rate>192000||!Number.isFinite(start)||start<0)throw new Error("PCM 输出采样率或播放位置无效");
    const url=new URL(await this.mediaUrl(trackId,signal));
    url.pathname=url.pathname.replace(/\/media\/[^/]+$/,"/pcm/"+encodeURIComponent(trackId));
    url.searchParams.set("rate",String(rate));url.searchParams.set("start",String(start));return url.href;
  }
  /** Native clients only. Verify X-RMusic-Native-FLAC before decoding; caps are chosen from source evidence and output capability. */
  async nativeFlacUrl(trackId:string,format:{sample_rate:number;bit_depth:16|24|32;channels:1|2;start?:number},signal?:AbortSignal){
    const {sample_rate:rate,bit_depth:bits,channels,start=0}=format;
    if(!Number.isInteger(rate)||rate<8000||rate>768000||![16,24,32].includes(bits)||![1,2].includes(channels)||!Number.isFinite(start)||start<0)throw new Error("原生 FLAC 流规格或播放位置无效");
    const url=new URL(await this.mediaUrl(trackId,signal));
    url.pathname=url.pathname.replace(/\/media\/[^/]+$/,"/native-flac/"+encodeURIComponent(trackId));
    url.searchParams.set("rate",String(rate));url.searchParams.set("bits",String(bits));url.searchParams.set("channels",String(channels));url.searchParams.set("start",String(start));return url.href;
  }
  /** Explicit lossy mobile delivery. Core must allow mobile_opus_enabled. Never enables a global policy. */
  async mobileOpusUrl(trackId:string,options:{bitrate?:number;channels?:1|2;start?:number}={},signal?:AbortSignal){
    const {bitrate=160,channels=2,start=0}=options;
    if(!Number.isInteger(bitrate)||bitrate<48||bitrate>320||![1,2].includes(channels)||!Number.isFinite(start)||start<0)throw new Error("Opus流配置无效");
    const url=new URL(await this.mediaUrl(trackId,signal));url.pathname=url.pathname.replace(/\/media\/[^/]+$/,"/mobile-opus/"+encodeURIComponent(trackId));
    url.searchParams.set("accept_lossy","true");url.searchParams.set("bitrate",String(bitrate));url.searchParams.set("channels",String(channels));url.searchParams.set("start",String(start));return url.href;
  }
  async aacPcmUrl(trackId:string,rate:number,start=0,signal?:AbortSignal){
    const url=new URL(await this.pcmUrl(trackId,rate,start,signal));url.pathname=url.pathname.replace(/\/pcm\//,"/transcode/");
    url.searchParams.delete("rate");url.searchParams.set("pcm_rate",String(rate));url.searchParams.set("bitrate","128");return url.href;
  }
  async events(onEvent: (event: { type: string; data: unknown }) => void,signal?:AbortSignal) {
    const { ticket } = await this.request<{ ticket: string }>("/events/ticket",{signal});
    signal?.throwIfAborted();
    const url = new URL(this.url("/api/v1/events"), window.location.href);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.searchParams.set("ticket", ticket);
    url.searchParams.set("zone_view","state");
    const socket = new WebSocket(url);
    socket.onmessage = (event) => {
      try {
        onEvent(JSON.parse(event.data));
      } catch {
        /* Ignore malformed event without changing authenticated state. */
      }
    };
    return socket;
  }
}
