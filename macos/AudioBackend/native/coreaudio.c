#include "coreaudio.h"
#include "feed_wake.h"
#include "feed_priority.h"
#include <AudioUnit/AudioUnit.h>
#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/CoreAudio.h>
#include <CoreAudio/HostTime.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <math.h>
#include <stdio.h>

#define RM_INVALID -70001
#define RM_FORMAT -70002
#define RM_TIMELINE -70003
#define RM_BUSY -70005
#define SPANS 2048

typedef struct { uint64_t start_ns, first; uint32_t frames; } Span;
struct RMOutput {
    RMFeedWake wake;
    RMFeedPriority priority;
    AudioDeviceID device;
    AudioUnit unit;
    uint32_t rate, channels, width, precision, capacity, route_bits, route_float;
    Float64 original_rate;
    uint64_t latency_ns;
    int hog, exclusive, suspended, changed_rate, initialized, started, listening, follows_default, default_listening;
    uint8_t *ring;
    _Atomic uint64_t read, write, delivered, underrun_frame;
    _Atomic uint64_t last_write_ns;
    _Atomic uint64_t start_presentation_ns;
    uint64_t history_floor_ns;
    uint32_t scheduled_output;
    uint64_t clock_query_ns;
    RMProgress paired_clock;
    RMFeedDiagnostic feed_diagnostic;
    _Atomic uint32_t paused, eof, underrun, dirty, quantum;
    _Atomic int32_t error;
    Span spans[SPANS];
    _Atomic uint32_t span_read, span_write;
    uint64_t played;
    // Only rm_progress's control thread accesses this last coherent callback snapshot.
    // A seqlock collision is not a device clock reset; retain its original timestamps.
    uint64_t observed_ns, observed_frame, observed_host;
    _Atomic uint64_t stamp_sequence, stamp_ns, stamp_frame, stamp_host;
};
static AudioObjectPropertyAddress prop(AudioObjectPropertySelector selector, AudioObjectPropertyScope scope) {
    return (AudioObjectPropertyAddress){selector,scope,kAudioObjectPropertyElementMain};
}
static OSStatus get(AudioObjectID id, AudioObjectPropertySelector selector, AudioObjectPropertyScope scope, void *value, UInt32 size) {
    AudioObjectPropertyAddress p=prop(selector,scope);
    return AudioObjectGetPropertyData(id,&p,0,NULL,&size,value);
}
static OSStatus set(AudioObjectID id, AudioObjectPropertySelector selector, void *value, UInt32 size) {
    AudioObjectPropertyAddress p=prop(selector,kAudioObjectPropertyScopeGlobal);
    return AudioObjectSetPropertyData(id,&p,0,NULL,size,value);
}
// HAL property writes can complete after SetPropertyData returns. A replacement session must
// not open until the previous session's restoration is observed and stable.
static OSStatus settle_rate(AudioDeviceID device,Float64 expected,Float64 *actual) {
    unsigned stable=0;
    for(unsigned i=0;i<200;i++) {
        OSStatus status=get(device,kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal,actual,sizeof(*actual));
        if(status)return status;
        stable=fabs(*actual-expected)<0.5?stable+1:0;
        if(stable>=3)return noErr;
        usleep(10000);
    }
    return RM_FORMAT;
}
static void string_property(AudioObjectID id, AudioObjectPropertySelector selector, char *out, size_t size) {
    CFStringRef s=NULL;
    if (!get(id,selector,kAudioObjectPropertyScopeGlobal,&s,sizeof(s)) && s) {
        CFStringGetCString(s,out,(CFIndex)size,kCFStringEncodingUTF8); CFRelease(s);
    }
}
static uint32_t channel_count(AudioDeviceID id) {
    AudioObjectPropertyAddress p=prop(kAudioDevicePropertyStreamConfiguration,kAudioObjectPropertyScopeOutput);
    UInt32 size=0;
    if(AudioObjectGetPropertyDataSize(id,&p,0,NULL,&size) || size<sizeof(AudioBufferList) || size>65536) return 0;
    AudioBufferList *list=calloc(1,size); if(!list) return 0;
    uint32_t count=0;
    if(!AudioObjectGetPropertyData(id,&p,0,NULL,&size,list)) for(UInt32 i=0;i<list->mNumberBuffers;i++) count+=list->mBuffers[i].mNumberChannels;
    free(list); return count;
}
int32_t rm_devices(RMDevice *items,uint32_t capacity,uint32_t *count) {
    AudioObjectPropertyAddress p=prop(kAudioHardwarePropertyDevices,kAudioObjectPropertyScopeGlobal);
    UInt32 size=0; OSStatus status=AudioObjectGetPropertyDataSize(kAudioObjectSystemObject,&p,0,NULL,&size);
    if(status) return status;
    if(size>65536) return RM_INVALID;
    AudioDeviceID *ids=calloc(1,size?size:1); if(!ids) return -70004;
    status=AudioObjectGetPropertyData(kAudioObjectSystemObject,&p,0,NULL,&size,ids);
    AudioDeviceID default_id=0;
    get(kAudioObjectSystemObject,kAudioHardwarePropertyDefaultOutputDevice,kAudioObjectPropertyScopeGlobal,&default_id,sizeof(default_id));
    *count=0;
    if(!status) for(UInt32 i=0;i<size/sizeof(*ids);i++) {
        uint32_t channels=channel_count(ids[i]); if(!channels) continue;
        if(*count>=capacity) { status=RM_INVALID; break; }
        RMDevice *item=&items[(*count)++]; memset(item,0,sizeof(*item));
        string_property(ids[i],kAudioDevicePropertyDeviceUID,item->uid,sizeof(item->uid));
        string_property(ids[i],kAudioObjectPropertyName,item->name,sizeof(item->name));
        Float64 rate=0; get(ids[i],kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal,&rate,sizeof(rate));
        item->rate=(uint32_t)llround(rate); item->channels=channels; item->is_default=ids[i]==default_id;
    }
    free(ids); return status;
}
static OSStatus changed(AudioObjectID id,UInt32 count,const AudioObjectPropertyAddress *addresses,void *context) {
    (void)id;(void)count;(void)addresses; atomic_store_explicit(&((RMOutput*)context)->dirty,1,memory_order_release); return noErr;
}
// This callback only copies pre-rendered samples and publishes bounded atomic clock records.
static OSStatus render_frames(void *context,AudioUnitRenderActionFlags *flags,const AudioTimeStamp *time,UInt32 bus,UInt32 frames,AudioBufferList *buffers) {
    (void)flags;(void)bus; RMOutput *o=context;
    atomic_store_explicit(&o->quantum,frames,memory_order_relaxed);
    for(UInt32 i=0;i<buffers->mNumberBuffers;i++) if(buffers->mBuffers[i].mData) memset(buffers->mBuffers[i].mData,0,buffers->mBuffers[i].mDataByteSize);
    if(buffers->mNumberBuffers!=1 || !buffers->mBuffers[0].mData || buffers->mBuffers[0].mDataByteSize<frames*o->width) {atomic_store(&o->error,-70006);return noErr;}
    if(!(time->mFlags&kAudioTimeStampHostTimeValid) || !(time->mFlags&kAudioTimeStampSampleTimeValid) || !isfinite(time->mSampleTime)) {atomic_store(&o->error,-70009);return noErr;}
    uint64_t ns=AudioConvertHostTimeToNanos(time->mHostTime);
    uint64_t previous_ns=atomic_load_explicit(&o->stamp_ns,memory_order_relaxed);
    if(previous_ns && (ns<=previous_ns || time->mSampleTime<=atomic_load_explicit(&o->stamp_frame,memory_order_relaxed))) {atomic_store(&o->error,-70010);return noErr;}
    atomic_fetch_add_explicit(&o->stamp_sequence,1,memory_order_acq_rel);
    atomic_store_explicit(&o->stamp_ns,ns,memory_order_relaxed);
    atomic_store_explicit(&o->stamp_frame,(uint64_t)fmax(0,time->mSampleTime),memory_order_relaxed);
    atomic_store_explicit(&o->stamp_host,AudioConvertHostTimeToNanos(AudioGetCurrentHostTime()),memory_order_relaxed);
    atomic_fetch_add_explicit(&o->stamp_sequence,1,memory_order_release);
    if(atomic_load_explicit(&o->paused,memory_order_acquire) || atomic_load(&o->error)) return noErr;
    uint64_t read=atomic_load_explicit(&o->read,memory_order_relaxed),write=atomic_load_explicit(&o->write,memory_order_acquire);
    uint64_t deadline=atomic_load_explicit(&o->start_presentation_ns,memory_order_acquire);
    uint64_t presentation=ns+o->latency_ns;
    uint32_t skip=deadline>presentation?(uint32_t)fmin(frames,((__uint128_t)(deadline-presentation)*o->rate+999999999)/1000000000):0;
    if(skip==frames)return noErr; // Scheduled silence is not missing programme data.
    uint32_t wanted=frames-skip;
    uint32_t n=(uint32_t)fmin(wanted,write-read);
    if(n) {
        uint32_t sw=atomic_load_explicit(&o->span_write,memory_order_relaxed),sr=atomic_load_explicit(&o->span_read,memory_order_acquire);
        if(sw-sr>=SPANS) {atomic_store(&o->error,-70011);return noErr;}
        uint32_t offset=(uint32_t)(read%o->capacity),first=n<o->capacity-offset?n:o->capacity-offset;
        uint8_t *target=(uint8_t*)buffers->mBuffers[0].mData+(size_t)skip*o->width;
        memcpy(target,o->ring+(size_t)offset*o->width,(size_t)first*o->width);
        memcpy(target+(size_t)first*o->width,o->ring,(size_t)(n-first)*o->width);
        uint64_t delivered=atomic_load_explicit(&o->delivered,memory_order_relaxed);
        o->spans[sw%SPANS]=(Span){presentation+(uint64_t)skip*1000000000/o->rate,delivered,n};
        atomic_store_explicit(&o->delivered,delivered+n,memory_order_relaxed);
        atomic_store_explicit(&o->start_presentation_ns,0,memory_order_release);
        atomic_store_explicit(&o->span_write,sw+1,memory_order_release);
        atomic_store_explicit(&o->read,read+n,memory_order_release);
    }
    if(n<wanted && !atomic_load_explicit(&o->eof,memory_order_acquire) && !atomic_load(&o->underrun)) {
        uint64_t host=atomic_load_explicit(&o->stamp_host,memory_order_relaxed),last=atomic_load_explicit(&o->last_write_ns,memory_order_acquire);
        o->feed_diagnostic=(RMFeedDiagnostic){last&&host>last?host-last:0,host>ns?host-ns:0,(uint32_t)(write-read),frames};
        atomic_store(&o->underrun_frame,atomic_load(&o->delivered));
        atomic_store_explicit(&o->underrun,1,memory_order_release);
    }
    return noErr;
}
static OSStatus render(void *context,AudioUnitRenderActionFlags *flags,const AudioTimeStamp *time,UInt32 bus,UInt32 frames,AudioBufferList *buffers) {
    OSStatus status=render_frames(context,flags,time,bus,frames,buffers);
    // Publish consumption first, then wake the producer. No decoding, allocation,
    // mutex, Swift call or logging on the callback; all paths wake the worker.
    rm_feed_wake_signal(&((RMOutput*)context)->wake);
    return status;
}
AudioDeviceID rm_find_device(const char *uid) {
    AudioDeviceID device=0;
    if(!uid || !*uid || !strcmp(uid,RM_SYSTEM_DEFAULT_UID)) {get(kAudioObjectSystemObject,kAudioHardwarePropertyDefaultOutputDevice,kAudioObjectPropertyScopeGlobal,&device,sizeof(device));return device;}
    CFStringRef text=CFStringCreateWithCString(NULL,uid,kCFStringEncodingUTF8); if(!text) return 0;
    AudioObjectPropertyAddress address=prop(kAudioHardwarePropertyTranslateUIDToDevice,kAudioObjectPropertyScopeGlobal);
    UInt32 size=sizeof(device);
    OSStatus status=AudioObjectGetPropertyData(kAudioObjectSystemObject,&address,sizeof(text),&text,&size,&device);
    CFRelease(text);return status?0:device;
}
static OSStatus acquire_hog(RMOutput *o) {
    pid_t owner=-1;OSStatus status=get(o->device,kAudioDevicePropertyHogMode,kAudioObjectPropertyScopeGlobal,&owner,sizeof(owner));if(status)return status;
    if(owner!=-1)return RM_BUSY;
    status=set(o->device,kAudioDevicePropertyHogMode,&owner,sizeof(owner));if(status)return status;
    // HAL toggles ownership. Verify the property itself, not a driver's mutation of the input pointer.
    pid_t actual=-1;status=get(o->device,kAudioDevicePropertyHogMode,kAudioObjectPropertyScopeGlobal,&actual,sizeof(actual));
    if(actual==getpid()||owner==getpid())o->hog=1;
    if(status)return status;
    return actual==getpid()?noErr:RM_BUSY;
}
static void release_hog(RMOutput *o) {
    pid_t owner=-1;if(o->hog&&!get(o->device,kAudioDevicePropertyHogMode,kAudioObjectPropertyScopeGlobal,&owner,sizeof(owner))&&owner==getpid())
        set(o->device,kAudioDevicePropertyHogMode,&owner,sizeof(owner));
    o->hog=0;
}
static void restore_rate(RMOutput *o) {
    Float64 current=0;
    if(o->changed_rate&&!get(o->device,kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal,&current,sizeof(current))&&fabs(current-o->rate)<0.5)
        if(!set(o->device,kAudioDevicePropertyNominalSampleRate,&o->original_rate,sizeof(o->original_rate)))settle_rate(o->device,o->original_rate,&current);
    o->changed_rate=0;
}
int32_t rm_open(const char *uid,uint32_t rate,uint32_t channels,uint32_t exclusive,uint32_t bits,uint32_t floating,uint32_t buffer_ms,RMOutput **output) {
    *output=NULL;
    if(rate<8000 || rate>768000 || channels<1 || channels>32 || buffer_ms<20 || buffer_ms>2000) return RM_FORMAT;
    RMOutput *o=calloc(1,sizeof(*o));if(!o) return -70004;
    OSStatus status=rm_feed_wake_create(&o->wake);if(status)goto fail;
    status=RM_INVALID; o->follows_default=uid&&!strcmp(uid,RM_SYSTEM_DEFAULT_UID);o->device=rm_find_device(uid);if(!o->device || channel_count(o->device)<channels) goto fail;
    o->rate=rate; o->channels=channels; o->width=channels*(floating?8:4);o->exclusive=exclusive!=0;
    status=get(o->device,kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal,&o->original_rate,sizeof(o->original_rate));if(status) goto fail;
    if(getenv("RMUSIC_AUDIO_DIAGNOSTICS"))fprintf(stderr,"HAL open %u requested=%u original=%.0f\n",o->device,rate,o->original_rate);
    if(exclusive) {
        status=acquire_hog(o);if(status)goto fail;
    }
    Float64 requested=rate;
    if(fabs(o->original_rate-requested)>0.5) {
        status=set(o->device,kAudioDevicePropertyNominalSampleRate,&requested,sizeof(requested));
        if(!status)o->changed_rate=1; // Shared mode may retain the current clock if the driver refuses.
    }

    Float64 actual=0;
    if(o->changed_rate) {status=settle_rate(o->device,rate,&actual);if(status&&exclusive)goto fail;}
    else {status=get(o->device,kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal,&actual,sizeof(actual));if(status)goto fail;}
    if(!isfinite(actual)||actual<8000||actual>768000||(exclusive&&fabs(actual-rate)>0.5)) {status=RM_FORMAT;goto fail;}
    o->rate=(uint32_t)llround(actual);
    if(getenv("RMUSIC_AUDIO_DIAGNOSTICS"))fprintf(stderr,"HAL negotiated %u rate=%u changed=%d\n",o->device,o->rate,o->changed_rate);
    (void)bits;
    AudioComponentDescription description={kAudioUnitType_Output,kAudioUnitSubType_HALOutput,kAudioUnitManufacturer_Apple,0,0};
    AudioComponent component=AudioComponentFindNext(NULL,&description);if(!component) {status=RM_INVALID;goto fail;}
    status=AudioComponentInstanceNew(component,&o->unit);if(status) goto fail;
    UInt32 one=1,zero=0;
    status=AudioUnitSetProperty(o->unit,kAudioOutputUnitProperty_EnableIO,kAudioUnitScope_Output,0,&one,sizeof(one));if(status) goto fail;
    status=AudioUnitSetProperty(o->unit,kAudioOutputUnitProperty_EnableIO,kAudioUnitScope_Input,1,&zero,sizeof(zero));if(status) goto fail;
    status=AudioUnitSetProperty(o->unit,kAudioOutputUnitProperty_CurrentDevice,kAudioUnitScope_Global,0,&o->device,sizeof(o->device));if(status) goto fail;
    AudioStreamBasicDescription hardware={0};UInt32 size=sizeof(hardware);
    status=AudioUnitGetProperty(o->unit,kAudioUnitProperty_StreamFormat,kAudioUnitScope_Output,0,&hardware,&size);if(status) goto fail;
    o->route_bits=hardware.mBitsPerChannel;o->route_float=(hardware.mFormatFlags&kAudioFormatFlagIsFloat)!=0;
    o->precision=hardware.mFormatFlags&kAudioFormatFlagIsFloat?(hardware.mBitsPerChannel==64?32:24):hardware.mBitsPerChannel;
    if(o->precision==0 || o->precision>32) o->precision=32;
    AudioStreamBasicDescription client={o->rate,kAudioFormatLinearPCM,kAudioFormatFlagIsPacked | (floating?kAudioFormatFlagIsFloat:kAudioFormatFlagIsSignedInteger),o->width,1,o->width,channels,floating?64:32,0};
    status=AudioUnitSetProperty(o->unit,kAudioUnitProperty_StreamFormat,kAudioUnitScope_Input,0,&client,sizeof(client));if(status) goto fail;
    UInt32 max_frames=0;size=sizeof(max_frames);
    status=AudioUnitGetProperty(o->unit,kAudioUnitProperty_MaximumFramesPerSlice,kAudioUnitScope_Global,0,&max_frames,&size);if(status) goto fail;
    if(max_frames>65536) {status=RM_FORMAT;goto fail;}
    o->capacity=(uint32_t)((uint64_t)o->rate*buffer_ms/1000);if(o->capacity<max_frames*2) o->capacity=max_frames*2;
    UInt32 latency=0,safety=0;
    get(o->device,kAudioDevicePropertyLatency,kAudioObjectPropertyScopeOutput,&latency,sizeof(latency));
    get(o->device,kAudioDevicePropertySafetyOffset,kAudioObjectPropertyScopeOutput,&safety,sizeof(safety));
    o->latency_ns=((uint64_t)latency+safety)*1000000000/o->rate;
    if(exclusive&&o->capacity<max_frames*2+latency+safety)o->capacity=max_frames*2+latency+safety;
    o->ring=calloc(o->capacity,o->width);if(!o->ring) {status=-70004;goto fail;}
    AURenderCallbackStruct callback={render,o};
    status=AudioUnitSetProperty(o->unit,kAudioUnitProperty_SetRenderCallback,kAudioUnitScope_Input,0,&callback,sizeof(callback));if(status) goto fail;
    status=AudioUnitInitialize(o->unit);if(status) goto fail;o->initialized=1;
    AudioObjectPropertySelector selectors[]={kAudioDevicePropertyDeviceIsAlive,kAudioDevicePropertyNominalSampleRate,kAudioDevicePropertyStreamConfiguration};
    for(int i=0;i<3;i++){AudioObjectPropertyAddress p=prop(selectors[i],i==2?kAudioObjectPropertyScopeOutput:kAudioObjectPropertyScopeGlobal);status=AudioObjectAddPropertyListener(o->device,&p,changed,o);if(status) goto fail;o->listening++;}
    if(o->follows_default) {
        AudioObjectPropertyAddress p=prop(kAudioHardwarePropertyDefaultOutputDevice,kAudioObjectPropertyScopeGlobal);
        status=AudioObjectAddPropertyListener(kAudioObjectSystemObject,&p,changed,o);if(status)goto fail;o->default_listening=1;
        if(rm_find_device(RM_SYSTEM_DEFAULT_UID)!=o->device){status=-70012;goto fail;}
    }
    *output=o;return noErr;
fail: rm_close(o);return status;
}
uint32_t rm_rate(RMOutput *o) {return o->rate;}
uint32_t rm_precision(RMOutput *o) {return o->precision;}
uint32_t rm_route_bits(RMOutput *o) {return o->route_bits;}
uint32_t rm_route_float(RMOutput *o) {return o->route_float;}
static uint32_t write_packet(RMOutput *o,const uint8_t *bytes,uint32_t frames,int final) {
    uint64_t write=atomic_load_explicit(&o->write,memory_order_relaxed),read=o->exclusive?o->played:atomic_load_explicit(&o->read,memory_order_acquire);
    uint32_t room=o->capacity-(uint32_t)(write-read),n=frames<room?frames:room;
    uint32_t offset=(uint32_t)(write%o->capacity),first=n<o->capacity-offset?n:o->capacity-offset;
    memcpy(o->ring+(size_t)offset*o->width,bytes,(size_t)first*o->width);
    memcpy(o->ring,bytes+(size_t)first*o->width,(size_t)(n-first)*o->width);
    if(n)atomic_store_explicit(&o->last_write_ns,AudioConvertHostTimeToNanos(AudioGetCurrentHostTime()),memory_order_release);
    if(final&&n==frames)atomic_store_explicit(&o->eof,1,memory_order_release);
    atomic_store_explicit(&o->write,write+n,memory_order_release);return n;
}
uint32_t rm_write(RMOutput *o,const uint8_t *bytes,uint32_t frames){return write_packet(o,bytes,frames,0);}
uint32_t rm_write_final(RMOutput *o,const uint8_t *bytes,uint32_t frames){return write_packet(o,bytes,frames,1);}
int32_t rm_start_at(RMOutput *o,uint64_t presentation_ns) {
    if(o->started)return RM_INVALID;
    if(rm_feed_priority_init(&o->priority,o->unit))return -70013;
    uint64_t now=AudioConvertHostTimeToNanos(AudioGetCurrentHostTime());
    if(presentation_ns && (presentation_ns<=now || presentation_ns-now>10000000000ULL))return RM_INVALID;
    o->scheduled_output=presentation_ns!=0;
    atomic_store_explicit(&o->start_presentation_ns,presentation_ns,memory_order_release);
    OSStatus status=AudioOutputUnitStart(o->unit);if(!status)o->started=1;return status;
}
int32_t rm_start(RMOutput *o){return rm_start_at(o,0);}

void rm_pause(RMOutput *o,uint32_t paused) {atomic_store_explicit(&o->paused,paused,memory_order_release);}
int32_t rm_wait(RMOutput *o,uint32_t milliseconds) {
    // Before start there is no HAL callback yet; only priming may use a timer.
    uint32_t timeout=o->started?milliseconds:2;
    if(o->started&&!atomic_load_explicit(&o->paused,memory_order_acquire) && rm_feed_priority_begin(&o->priority,atomic_load(&o->quantum),o->rate))return -70013;
    return rm_feed_wake_wait(&o->wake,timeout)==KERN_SUCCESS?0:-70013;
}
void rm_finish_cycle(RMOutput *o) {rm_feed_priority_end(&o->priority);}
uint64_t rm_underrun_frame(RMOutput *o) {return atomic_load(&o->underrun_frame);}
void rm_feed_diagnostic(RMOutput *o,RMFeedDiagnostic *diagnostic) {
    memset(diagnostic,0,sizeof(*diagnostic));
    if(atomic_load_explicit(&o->underrun,memory_order_acquire))*diagnostic=o->feed_diagnostic;
}
void rm_finish(RMOutput *o) {atomic_store_explicit(&o->eof,1,memory_order_release);}
int32_t rm_progress(RMOutput *o,RMProgress *p) {
    int32_t error=atomic_load(&o->error);if(error)return error;
    if(o->suspended) {
        memset(p,0,sizeof(*p));p->played=o->played;p->queued=atomic_load_explicit(&o->write,memory_order_acquire)-o->played;
        p->capacity=o->capacity;p->quantum=atomic_load_explicit(&o->quantum,memory_order_relaxed);return noErr;
    }
    if(atomic_exchange_explicit(&o->dirty,0,memory_order_acq_rel)) {
        rm_finish_cycle(o); // HAL property reads are not part of the copy deadline.
        // Existing recovery closes this route and renegotiates the new default from Core position.
        if(o->follows_default&&rm_find_device(RM_SYSTEM_DEFAULT_UID)!=o->device)return -70012;
        UInt32 alive=0;Float64 rate=0;
        if(get(o->device,kAudioDevicePropertyDeviceIsAlive,kAudioObjectPropertyScopeGlobal,&alive,sizeof(alive))||!alive) return RM_INVALID;
        if(get(o->device,kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal,&rate,sizeof(rate))||fabs(rate-o->rate)>0.5) {if(getenv("RMUSIC_AUDIO_DIAGNOSTICS"))fprintf(stderr,"HAL clock mismatch %u expected=%u observed=%.0f\n",o->device,o->rate,rate);return -70007;}
        if(channel_count(o->device)<o->channels) return -70008;
    }
    memset(p,0,sizeof(*p));uint64_t now=AudioConvertHostTimeToNanos(AudioGetCurrentHostTime());
    uint32_t sr=atomic_load_explicit(&o->span_read,memory_order_relaxed),sw=atomic_load_explicit(&o->span_write,memory_order_acquire);
    while(sr!=sw) {
        Span span=o->spans[sr%SPANS];uint64_t end=span.start_ns+(uint64_t)span.frames*1000000000/o->rate;
        if(sw-sr<=SPANS/2 && (now<end || now-end<200000000))break;
        o->history_floor_ns=end;sr++;atomic_store_explicit(&o->span_read,sr,memory_order_release);
    }
    uint64_t heard=sr!=sw?o->spans[sr%SPANS].first:o->played;
    for(uint32_t i=sr;i!=sw;i++) {
        Span span=o->spans[i%SPANS];if(now<=span.start_ns)break;
        uint64_t frames=(uint64_t)(((__uint128_t)(now-span.start_ns)*o->rate)/1000000000);
        if(frames>span.frames)frames=span.frames;heard=span.first+frames;if(frames<span.frames)break;
    }
    if(heard>o->played)o->played=heard;
    // Paired HAL reads are needed only for scheduled groups, at most 50 times/second.
    // Reused samples keep their original read bracket; they are never re-timestamped.
    if(o->scheduled_output && (!o->clock_query_ns || now-o->clock_query_ns>=20000000)) {
        o->clock_query_ns=now;o->paired_clock.capture_valid=0;
    AudioTimeStamp capture={0};uint64_t before=AudioConvertHostTimeToNanos(AudioGetCurrentHostTime());
    OSStatus captured=o->device?AudioDeviceGetCurrentTime(o->device,&capture):kAudioHardwareUnsupportedOperationError;
    uint64_t after=AudioConvertHostTimeToNanos(AudioGetCurrentHostTime());
    if(!captured && (capture.mFlags&kAudioTimeStampHostTimeValid) && (capture.mFlags&kAudioTimeStampSampleTimeValid) && isfinite(capture.mSampleTime) && capture.mSampleTime>=0) {
        uint64_t at=AudioConvertHostTimeToNanos(capture.mHostTime);
        if(at<=after && after-at<=100000000 && at>=o->history_floor_ns) {
            uint64_t programme=sr!=sw?o->spans[sr%SPANS].first:o->played;
            for(uint32_t i=sr;i!=sw;i++){Span span=o->spans[i%SPANS];if(at<=span.start_ns)break;uint64_t n=(uint64_t)(((__uint128_t)(at-span.start_ns)*o->rate)/1000000000);if(n>span.frames)n=span.frames;programme=span.first+n;if(n<span.frames)break;}
            o->paired_clock.capture_frame=(uint64_t)capture.mSampleTime;o->paired_clock.capture_ns=at;o->paired_clock.capture_before=before;o->paired_clock.capture_after=after;o->paired_clock.programme_frames=programme;o->paired_clock.capture_valid=1;
        }
    }
    }
    RMProgress *paired=&o->paired_clock;
    uint64_t clock_now=paired->capture_valid?AudioConvertHostTimeToNanos(AudioGetCurrentHostTime()):0;
    if(paired->capture_valid && clock_now>=paired->capture_after && clock_now-paired->capture_after<=100000000) {
        p->capture_frame=paired->capture_frame;p->capture_ns=paired->capture_ns;p->capture_before=paired->capture_before;p->capture_after=paired->capture_after;p->programme_frames=paired->programme_frames;p->capture_valid=1;
    }
    p->played=o->played;p->underrun=atomic_load(&o->underrun);p->running=o->started&&!atomic_load_explicit(&o->paused,memory_order_acquire);
    p->queued=atomic_load_explicit(&o->write,memory_order_acquire)-(o->exclusive?o->played:atomic_load_explicit(&o->read,memory_order_acquire));
    p->capacity=o->capacity;p->quantum=atomic_load_explicit(&o->quantum,memory_order_relaxed);
    for(int i=0;i<4;i++) {
        uint64_t before=atomic_load_explicit(&o->stamp_sequence,memory_order_acquire);if(before&1)continue;
        p->device_ns=atomic_load_explicit(&o->stamp_ns,memory_order_relaxed);p->device_frame=atomic_load_explicit(&o->stamp_frame,memory_order_relaxed);
        p->before_ns=p->after_ns=atomic_load_explicit(&o->stamp_host,memory_order_relaxed);
        if(before==atomic_load_explicit(&o->stamp_sequence,memory_order_acquire)) {
            o->observed_ns=p->device_ns;o->observed_frame=p->device_frame;o->observed_host=p->after_ns;
            return noErr;
        }
    }
    // The audio callback may be between its atomic stores. Do not fabricate a fresh
    // clock or stop healthy playback on scheduling contention. The existing 500 ms
    // stale-clock observation window still stops an output whose callbacks actually stop.
    if(getenv("RMUSIC_AUDIO_DIAGNOSTICS"))fprintf(stderr,"HAL snapshot busy; retained coherent clock at %llu\n",(unsigned long long)o->observed_ns);
    p->device_ns=o->observed_ns;p->device_frame=o->observed_frame;
    p->before_ns=p->after_ns=o->observed_host;
    return noErr;
}
int32_t rm_user_pause(RMOutput *o,uint32_t paused) {
    if(!o->exclusive||o->suspended==(paused!=0))return noErr;
    rm_feed_priority_close(&o->priority);
    if(paused) {
        atomic_store_explicit(&o->paused,1,memory_order_release);
        OSStatus status=o->started?AudioOutputUnitStop(o->unit):noErr;if(status)return status;
        RMProgress progress={0};status=rm_progress(o,&progress);if(status)return status;
        // Retain every unconfirmed PCM frame in the ring, including device latency.
        atomic_store_explicit(&o->read,o->played,memory_order_release);
        atomic_store_explicit(&o->delivered,o->played,memory_order_relaxed);
        atomic_store(&o->span_read,0);atomic_store(&o->span_write,0);o->history_floor_ns=0;
        if(o->initialized){status=AudioUnitUninitialize(o->unit);if(status)return status;o->initialized=0;}
        restore_rate(o);release_hog(o);o->suspended=1;
        fprintf(stderr,"CoreAudio exclusive output released for user pause: played=%llu\n",(unsigned long long)o->played);
        return noErr;
    }
    if(o->follows_default&&rm_find_device(RM_SYSTEM_DEFAULT_UID)!=o->device)return -70012;
    OSStatus status=get(o->device,kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal,&o->original_rate,sizeof(o->original_rate));if(status)return status;
    status=acquire_hog(o);if(status)return status;
    if(fabs(o->original_rate-o->rate)>0.5) {
        Float64 requested=o->rate;status=set(o->device,kAudioDevicePropertyNominalSampleRate,&requested,sizeof(requested));if(status)return status;o->changed_rate=1;
        status=settle_rate(o->device,requested,&requested);if(status)return status;
    }
    status=AudioUnitInitialize(o->unit);if(status)return status;o->initialized=1;
    memset(&o->paired_clock,0,sizeof(o->paired_clock));o->clock_query_ns=0;
    atomic_store(&o->stamp_sequence,0);atomic_store(&o->stamp_ns,0);atomic_store(&o->stamp_frame,0);atomic_store(&o->stamp_host,0);
    o->observed_ns=o->observed_frame=o->observed_host=0;atomic_store(&o->dirty,1);
    if(o->started) {
        if(rm_feed_priority_init(&o->priority,o->unit))return -70013;
        status=AudioOutputUnitStart(o->unit);if(status)return status;
    }
    o->suspended=0;
    fprintf(stderr,"CoreAudio exclusive output resumed: played=%llu\n",(unsigned long long)o->played);
    return noErr;
}
void rm_close(RMOutput *o) {
    if(!o)return;
    rm_feed_priority_close(&o->priority);
    if(o->started&&!o->suspended)AudioOutputUnitStop(o->unit);
    AudioObjectPropertySelector selectors[]={kAudioDevicePropertyDeviceIsAlive,kAudioDevicePropertyNominalSampleRate,kAudioDevicePropertyStreamConfiguration};
    for(int i=0;i<o->listening;i++){AudioObjectPropertyAddress p=prop(selectors[i],i==2?kAudioObjectPropertyScopeOutput:kAudioObjectPropertyScopeGlobal);AudioObjectRemovePropertyListener(o->device,&p,changed,o);}
    if(o->default_listening){AudioObjectPropertyAddress p=prop(kAudioHardwarePropertyDefaultOutputDevice,kAudioObjectPropertyScopeGlobal);AudioObjectRemovePropertyListener(kAudioObjectSystemObject,&p,changed,o);}
    if(o->initialized)AudioUnitUninitialize(o->unit);if(o->unit)AudioComponentInstanceDispose(o->unit);
    // Restore only a rate changed by this session, and never overwrite another application's later change.
    if(getenv("RMUSIC_AUDIO_DIAGNOSTICS"))fprintf(stderr,"HAL close %u rate=%u restore=%.0f changed=%d continuity_lost=%u\n",o->device,o->rate,o->original_rate,o->changed_rate,atomic_load_explicit(&o->underrun,memory_order_acquire));
    restore_rate(o);
    release_hog(o);
    rm_feed_wake_close(&o->wake);free(o->ring);free(o);
}
