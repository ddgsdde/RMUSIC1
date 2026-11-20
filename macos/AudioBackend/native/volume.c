#include "coreaudio.h"
#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <math.h>
#include <string.h>

#define RM_VOLUME_UNAVAILABLE -70006
#define RM_VOLUME_FAILED -70007
#define RM_VOLUME_CHANNELS 32

static AudioDeviceID volume_device(const char *uid) {
    if (!uid || !*uid) return 0; // Never fall back to the system default.
    if(!strcmp(uid,RM_SYSTEM_DEFAULT_UID)) {
        extern AudioDeviceID rm_find_device(const char *uid);
        return rm_find_device(uid);
    }
    AudioDeviceID device = 0;
    CFStringRef text = CFStringCreateWithCString(NULL, uid, kCFStringEncodingUTF8);
    if (!text) return 0;
    AudioObjectPropertyAddress address = {kAudioHardwarePropertyTranslateUIDToDevice, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain};
    UInt32 size = sizeof(device);
    OSStatus status = AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, sizeof(text), &text, &size, &device);
    CFRelease(text);
    return status ? 0 : device;
}
static AudioObjectPropertyAddress scalar(UInt32 channel) {
    return (AudioObjectPropertyAddress){kAudioDevicePropertyVolumeScalar, kAudioDevicePropertyScopeOutput, channel};
}
static int read_control(AudioDeviceID device, UInt32 channel, Float32 *value) {
    AudioObjectPropertyAddress address = scalar(channel);
    Boolean writable = false;
    UInt32 size = sizeof(*value);
    return AudioObjectHasProperty(device, &address)
        && !AudioObjectIsPropertySettable(device, &address, &writable) && writable
        && !AudioObjectGetPropertyData(device, &address, 0, NULL, &size, value)
        && isfinite(*value) && *value >= 0 && *value <= 1;
}
static UInt32 output_channels(AudioDeviceID device) {
    AudioObjectPropertyAddress address = {kAudioDevicePropertyStreamConfiguration, kAudioDevicePropertyScopeOutput, kAudioObjectPropertyElementMain};
    // Bounded stack storage: discovery must never allocate an unbounded channel list.
    union { AudioBufferList align; unsigned char bytes[4096]; } buffer;
    UInt32 size = sizeof(buffer);
    if (AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &buffer)) return 0;
    AudioBufferList *list = (AudioBufferList *)&buffer;
    if (size < offsetof(AudioBufferList, mBuffers) || list->mNumberBuffers > (size - offsetof(AudioBufferList, mBuffers)) / sizeof(AudioBuffer)) return 0;
    UInt32 count = 0;
    for (UInt32 i = 0; i < list->mNumberBuffers; i++) {
        if (list->mBuffers[i].mNumberChannels > RM_VOLUME_CHANNELS - count) return 0;
        count += list->mBuffers[i].mNumberChannels;
    }
    return count;
}
static UInt32 volume_controls(AudioDeviceID device, UInt32 *channels, Float32 *values) {
    if (read_control(device, kAudioObjectPropertyElementMain, values)) { channels[0] = kAudioObjectPropertyElementMain; return 1; }
    UInt32 count = output_channels(device);
    for (UInt32 i = 0; i < count; i++) {
        channels[i] = i + 1;
        if (!read_control(device, channels[i], &values[i])) return 0;
    }
    return count;
}
static AudioObjectPropertyAddress mute(UInt32 channel) {
    return (AudioObjectPropertyAddress){kAudioDevicePropertyMute, kAudioDevicePropertyScopeOutput, channel};
}
static int read_mute(AudioDeviceID device, UInt32 channel, UInt32 *value) {
    AudioObjectPropertyAddress address = mute(channel);
    UInt32 size = sizeof(*value);
    return AudioObjectHasProperty(device, &address)
        && !AudioObjectGetPropertyData(device, &address, 0, NULL, &size, value);
}
static UInt32 mute_controls(AudioDeviceID device, UInt32 *channels, UInt32 *states) {
    if (read_mute(device, kAudioObjectPropertyElementMain, states)) {
        channels[0] = kAudioObjectPropertyElementMain;
        return 1;
    }
    UInt32 count = output_channels(device);
    for (UInt32 i = 0; i < count; i++) {
        channels[i] = i + 1;
        if (!read_mute(device, channels[i], &states[i])) return 0;
    }
    return count;
}
static int all_muted(const UInt32 *states, UInt32 count) {
    if (!count) return 0;
    for (UInt32 i = 0; i < count; i++) if (!states[i]) return 0;
    return 1;
}
static int writable_mute(AudioDeviceID device, const UInt32 *channels, UInt32 count) {
    if (!count) return 0;
    for (UInt32 i = 0; i < count; i++) {
        AudioObjectPropertyAddress address = mute(channels[i]);
        Boolean writable = false;
        if (AudioObjectIsPropertySettable(device, &address, &writable) || !writable) return 0;
    }
    return 1;
}
static int32_t write_mute(AudioDeviceID device, const UInt32 *channels, const UInt32 *previous, UInt32 count, UInt32 value) {
    for (UInt32 i = 0; i < count; i++) {
        if (previous[i] == value) continue;
        AudioObjectPropertyAddress address = mute(channels[i]);
        if (AudioObjectSetPropertyData(device, &address, 0, NULL, sizeof(value), &value)) {
            int failed = 0;
            for (UInt32 j = 0; j <= i; j++) {
                if (previous[j] == value) continue;
                address = mute(channels[j]);
                if (AudioObjectSetPropertyData(device, &address, 0, NULL, sizeof(previous[j]), &previous[j])) failed = 1;
            }
            return failed ? RM_VOLUME_FAILED : RM_VOLUME_UNAVAILABLE;
        }
    }
    return 0;
}
static int restore_levels(AudioDeviceID device, const UInt32 *channels, const Float32 *values, UInt32 count) {
    int failed = 0;
    for (UInt32 i = 0; i < count; i++) {
        AudioObjectPropertyAddress address = scalar(channels[i]);
        if (AudioObjectSetPropertyData(device, &address, 0, NULL, sizeof(values[i]), &values[i])) failed = 1;
    }
    return failed;
}
int32_t rm_device_volume(const char *uid, RMDeviceVolume *state) {
    if (!state) return RM_VOLUME_UNAVAILABLE;
    memset(state, 0, sizeof(*state));
    AudioDeviceID device = volume_device(uid);
    if (!device) return RM_VOLUME_UNAVAILABLE;
    UInt32 channels[RM_VOLUME_CHANNELS]; Float32 values[RM_VOLUME_CHANNELS];
    UInt32 count = volume_controls(device, channels, values);
    state->supported = count > 0;
    UInt32 mute_channels[RM_VOLUME_CHANNELS], mute_states[RM_VOLUME_CHANNELS];
    UInt32 mute_count = mute_controls(device, mute_channels, mute_states);
    state->muted = all_muted(mute_states, mute_count);
    state->mute_supported = writable_mute(device, mute_channels, mute_count);
    for (UInt32 i = 0; i < count; i++) state->value = fmaxf(state->value, values[i]);
    return 0;
}
int32_t rm_set_device_volume(const char *uid, float value, RMDeviceVolume *state) {
    if (!isfinite(value) || value < 0 || value > 1 || !state) return RM_VOLUME_UNAVAILABLE;
    AudioDeviceID device = volume_device(uid);
    if (!device) return RM_VOLUME_UNAVAILABLE;
    UInt32 channels[RM_VOLUME_CHANNELS]; Float32 old[RM_VOLUME_CHANNELS];
    UInt32 count = volume_controls(device, channels, old);
    if (!count) return RM_VOLUME_UNAVAILABLE;
    UInt32 mute_channels[RM_VOLUME_CHANNELS], mute_states[RM_VOLUME_CHANNELS];
    UInt32 mute_count = value > 0 ? mute_controls(device, mute_channels, mute_states) : 0;
    if (!all_muted(mute_states, mute_count)) mute_count = 0;
    if (mute_count && !writable_mute(device, mute_channels, mute_count)) return RM_VOLUME_UNAVAILABLE;
    Float32 peak = 0;
    for (UInt32 i = 0; i < count; i++) peak = fmaxf(peak, old[i]);
    for (UInt32 i = 0; i < count; i++) {
        // Preserve existing channel balance when adjusting a multi-channel device.
        Float32 next = peak > 0 ? old[i] * value / peak : value;
        AudioObjectPropertyAddress address = scalar(channels[i]);
        if (AudioObjectSetPropertyData(device, &address, 0, NULL, sizeof(next), &next)) {
            return restore_levels(device, channels, old, i + 1) ? RM_VOLUME_FAILED : RM_VOLUME_UNAVAILABLE;
        }
    }
    // Install the requested level before unmuting so the old level cannot come through.
    if (mute_count) {
        int32_t status = write_mute(device, mute_channels, mute_states, mute_count, 0);
        if (status == RM_VOLUME_FAILED) return status;
        if (status) return restore_levels(device, channels, old, count) ? RM_VOLUME_FAILED : status;
    }
    return rm_device_volume(uid, state);
}

int32_t rm_set_device_mute(const char *uid, uint32_t value, RMDeviceVolume *state) {
    if (value > 1 || !state) return RM_VOLUME_UNAVAILABLE;
    AudioDeviceID device = volume_device(uid);
    if (!device) return RM_VOLUME_UNAVAILABLE;
    UInt32 channels[RM_VOLUME_CHANNELS], previous[RM_VOLUME_CHANNELS];
    UInt32 count = mute_controls(device, channels, previous);
    if (!writable_mute(device, channels, count)) return RM_VOLUME_UNAVAILABLE;
    int32_t status = write_mute(device, channels, previous, count, value);
    return status ? status : rm_device_volume(uid, state);
}
