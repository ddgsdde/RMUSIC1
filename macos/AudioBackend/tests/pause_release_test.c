// Synthetic HAL: preserve unconfirmed PCM while another application owns the device.
#include <AudioToolbox/AudioToolbox.h>
#include <CoreAudio/CoreAudio.h>
#include <assert.h>
#include <unistd.h>
static pid_t owner;
static Float64 nominal=44100;
static int starts,stops,initializations,uninitializations;
static OSStatus fake_get(AudioObjectID id,const AudioObjectPropertyAddress *a,UInt32 q,const void *v,UInt32 *size,void *out) {
    (void)id;(void)q;(void)v;(void)size;
    if(a->mSelector==kAudioDevicePropertyHogMode){*(pid_t*)out=owner;return 0;}
    if(a->mSelector==kAudioDevicePropertyNominalSampleRate){*(Float64*)out=nominal;return 0;}
    return -1;
}
static OSStatus fake_set(AudioObjectID id,const AudioObjectPropertyAddress *a,UInt32 q,const void *v,UInt32 size,const void *in) {
    (void)id;(void)q;(void)v;(void)size;
    if(a->mSelector==kAudioDevicePropertyHogMode){owner=owner==getpid()?-1:getpid();return 0;}
    if(a->mSelector==kAudioDevicePropertyNominalSampleRate){nominal=*(const Float64*)in;return 0;}
    return -1;
}
static OSStatus fake_start(AudioUnit unit){(void)unit;starts++;return 0;}
static OSStatus fake_stop(AudioUnit unit){(void)unit;stops++;return 0;}
static OSStatus fake_initialize(AudioUnit unit){(void)unit;initializations++;return 0;}
static OSStatus fake_uninitialize(AudioUnit unit){(void)unit;uninitializations++;return 0;}
static OSStatus fake_unit_get(AudioUnit unit,AudioUnitPropertyID property,AudioUnitScope scope,AudioUnitElement element,void *data,UInt32 *size){(void)unit;(void)property;(void)scope;(void)element;(void)data;(void)size;return -1;}
#define AudioObjectGetPropertyData fake_get
#define AudioObjectSetPropertyData fake_set
#define AudioOutputUnitStart fake_start
#define AudioOutputUnitStop fake_stop
#define AudioUnitInitialize fake_initialize
#define AudioUnitUninitialize fake_uninitialize
#define AudioUnitGetProperty fake_unit_get
#include "../native/coreaudio.c"
int main(void) {
    owner=getpid();RMOutput o={0};o.device=123;o.rate=44100;o.channels=2;o.width=8;o.capacity=32;
    o.exclusive=o.hog=o.initialized=o.started=o.changed_rate=1;o.original_rate=48000;o.ring=calloc(32,8);
    rm_pause(&o,1);assert(owner==getpid()&&!o.suspended&&stops==0);rm_pause(&o,0);
    int32_t input[72];for(int i=0;i<72;i++)input[i]=i*70001-1200000;
    assert(rm_write(&o,(uint8_t*)input,20)==20);
    atomic_store(&o.read,12);atomic_store(&o.delivered,12);o.played=4;
    // Device-delivered frames 4..11 are not confirmed heard and cannot be overwritten.
    assert(rm_write(&o,(uint8_t*)(input+40),20)==16);
    assert(rm_user_pause(&o,1)==0);assert(o.suspended&&owner==-1&&nominal==48000);
    assert(stops==1&&uninitializations==1&&!o.initialized);
    assert(atomic_load(&o.read)==4&&atomic_load(&o.delivered)==4&&o.played==4);
    for(int frame=4;frame<36;frame++)assert(!memcmp(o.ring+(frame%32)*8,input+frame*2,8));
    nominal=96000;owner=12345;assert(rm_user_pause(&o,0)==RM_BUSY);assert(o.suspended&&nominal==96000);
    owner=-1;assert(rm_user_pause(&o,0)==0);assert(owner==getpid()&&nominal==44100&&!o.suspended);
    assert(starts==1&&initializations==1&&o.played==4&&atomic_load(&o.read)==4);
    atomic_store(&o.dirty,0);assert(rm_user_pause(&o,1)==0);assert(owner==-1&&nominal==96000);
    free(o.ring);
    RMOutput shared={0};assert(rm_user_pause(&shared,1)==0&&!shared.suspended);
    RMOutput initial={0};initial.exclusive=initial.hog=initial.initialized=1;initial.device=123;initial.rate=44100;initial.original_rate=96000;initial.changed_rate=1;initial.capacity=32;
    owner=getpid();nominal=44100;int previous_starts=starts;
    assert(rm_user_pause(&initial,1)==0&&owner==-1&&initial.suspended);
    assert(rm_user_pause(&initial,0)==0&&owner==getpid()&&starts==previous_starts);
    atomic_store(&initial.dirty,0);assert(rm_user_pause(&initial,1)==0&&owner==-1);
    puts("PASS: exclusive pause releases HAL/hog; unconfirmed PCM retained byte for byte; foreign owner respected; resume restores exact format; shared pause unchanged (synthetic HAL)");
}
