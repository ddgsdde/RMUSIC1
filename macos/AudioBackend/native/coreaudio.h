#pragma once
#include <stdint.h>
#include <stddef.h>
#define RM_SYSTEM_DEFAULT_UID "rmusic:system-default"
typedef struct RMOutput RMOutput;
typedef struct { char uid[512]; char name[512]; uint32_t rate, channels, is_default; } RMDevice;
typedef struct { uint64_t played, device_frame, device_ns, before_ns, after_ns, queued; uint32_t running, underrun, capacity, quantum; uint64_t capture_frame,capture_ns,capture_before,capture_after,programme_frames; uint32_t capture_valid; } RMProgress;
typedef struct { uint64_t producer_gap_ns, callback_late_ns; uint32_t queued, requested; } RMFeedDiagnostic;
void rm_feed_diagnostic(RMOutput *output, RMFeedDiagnostic *diagnostic);
int32_t rm_devices(RMDevice *items, uint32_t capacity, uint32_t *count);
int32_t rm_open(const char *uid, uint32_t rate, uint32_t channels, uint32_t exclusive, uint32_t bits, uint32_t floating, uint32_t buffer_ms, RMOutput **output);
uint32_t rm_rate(RMOutput *output);
uint32_t rm_precision(RMOutput *output);
uint32_t rm_route_bits(RMOutput *output);
uint32_t rm_route_float(RMOutput *output);
uint32_t rm_write(RMOutput *output, const uint8_t *bytes, uint32_t frames);
uint32_t rm_write_final(RMOutput *output,const uint8_t *bytes,uint32_t frames);
int32_t rm_start(RMOutput *output);
int32_t rm_start_at(RMOutput *output,uint64_t presentation_ns);
void rm_pause(RMOutput *output, uint32_t paused);
int32_t rm_user_pause(RMOutput *output, uint32_t paused);
int32_t rm_wait(RMOutput *output,uint32_t milliseconds);
void rm_finish_cycle(RMOutput *output);
uint64_t rm_underrun_frame(RMOutput *output);
void rm_finish(RMOutput *output);
int32_t rm_progress(RMOutput *output, RMProgress *progress);
void rm_close(RMOutput *output);

// Device controls are queried and written only on the non-audio control path.
typedef struct { uint32_t supported; float value; uint32_t muted, mute_supported; } RMDeviceVolume;
int32_t rm_device_volume(const char *uid, RMDeviceVolume *state);
int32_t rm_set_device_volume(const char *uid, float value, RMDeviceVolume *state);
int32_t rm_set_device_mute(const char *uid, uint32_t value, RMDeviceVolume *state);

typedef struct { double min_rate,max_rate; uint32_t bits,channels,floating; } RMFormat;
typedef struct { uint32_t count,rate_writable,exclusive; } RMCapabilities;
int32_t rm_capabilities(const char *uid, RMFormat *formats, uint32_t capacity, RMCapabilities *result);
