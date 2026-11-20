// Synthetic CoreAudio properties: no real device is read or changed by this test.
#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <assert.h>
#include <math.h>
#include <stdio.h>
static int writes, master, writable=1, fail_channel;
static UInt32 mute_flags[3];
static int mute_layout, mute_writable=1;
static Float32 levels[3]={.5f,.25f,.5f};
static Boolean fake_has(AudioObjectID id,const AudioObjectPropertyAddress *address) { if (id != 123) return false;
    if (address->mSelector == kAudioDevicePropertyMute)
        return mute_layout == 1 ? address->mElement == 0 : mute_layout == 2 && address->mElement != 0;
    return address->mElement != 0 || master; }
static OSStatus fake_settable(AudioObjectID id,const AudioObjectPropertyAddress *address,Boolean *value) { (void)id;*value=address->mSelector==kAudioDevicePropertyMute ? mute_writable : writable;return 0; }
static OSStatus fake_get(AudioObjectID id,const AudioObjectPropertyAddress *address,UInt32 qualifierSize,const void *qualifier,UInt32 *size,void *out) {
    (void)id;(void)qualifierSize;
    if(address->mSelector==kAudioHardwarePropertyTranslateUIDToDevice){
        CFStringRef uid=*(CFStringRef *)qualifier;
        if(!CFEqual(uid,CFSTR("fixture")))return -1;
        *(AudioDeviceID*)out=123;return 0;
    }
    if(address->mSelector==kAudioDevicePropertyStreamConfiguration){
        AudioBufferList *list=out;list->mNumberBuffers=1;list->mBuffers[0].mNumberChannels=2;*size=sizeof(AudioBufferList);return 0;
    }
    if (address->mSelector==kAudioDevicePropertyMute) { *(UInt32*)out=mute_flags[address->mElement]; return 0; }
    *(Float32*)out=levels[address->mElement];return 0;
}
static OSStatus fake_set(AudioObjectID id,const AudioObjectPropertyAddress *address,UInt32 qualifierSize,const void *qualifier,UInt32 size,const void *value) {
    (void)id;(void)qualifierSize;(void)qualifier;(void)size;writes++;
    if((int)address->mElement==fail_channel){fail_channel=-1;return -1;}
    if (address->mSelector==kAudioDevicePropertyMute) {
        // Attenuation must already be installed before a device can become audible.
        if (!*(const UInt32*)value) assert(levels[address->mElement] > 0 && levels[address->mElement] <= .8f);
        mute_flags[address->mElement]=*(const UInt32*)value;return 0;
    }
    levels[address->mElement]=*(const Float32*)value;return 0;
}
#define AudioObjectHasProperty fake_has
#define AudioObjectIsPropertySettable fake_settable
#define AudioObjectGetPropertyData fake_get
#define AudioObjectSetPropertyData fake_set
#include "../native/volume.c"
int main(void){
    RMDeviceVolume state;master=1;fail_channel=-1;
    assert(!rm_device_volume("fixture",&state)&&state.supported&&fabsf(state.value-.5f)<.001f&&writes==0);
    assert(rm_set_device_volume("",.8f,&state)&&writes==0);
    assert(rm_set_device_volume("missing",.8f,&state)&&writes==0);
    assert(rm_set_device_volume("fixture",NAN,&state)&&writes==0);
    writable=0;assert(!rm_device_volume("fixture",&state)&&!state.supported&&writes==0);
    assert(rm_set_device_volume("fixture",.8f,&state)&&writes==0);
    writable=1;assert(!rm_set_device_volume("fixture",.8f,&state)&&fabsf(state.value-.8f)<.001f);
    master=0;levels[1]=.25f;levels[2]=.5f;
    assert(!rm_set_device_volume("fixture",.8f,&state)&&fabsf(levels[1]-.4f)<.001f&&fabsf(levels[2]-.8f)<.001f);
    Float32 left=levels[1],right=levels[2];fail_channel=2;
    assert(rm_set_device_volume("fixture",.6f,&state));
    assert(levels[1]==left&&levels[2]==right); // partial channel write rolled back
    assert(!rm_set_device_volume("fixture",1,&state)&&state.value==1);
    master=1;mute_layout=1;mute_flags[0]=1;
    assert(!rm_set_device_volume("fixture",0,&state)&&mute_flags[0]==1);
    assert(!rm_set_device_volume("fixture",.3f,&state)&&mute_flags[0]==0&&fabsf(state.value-.3f)<.001f);
    assert(state.mute_supported && !state.muted);
    assert(!rm_set_device_mute("fixture",1,&state)&&state.muted&&fabsf(state.value-.3f)<.001f);
    assert(!rm_set_device_mute("fixture",0,&state)&&!state.muted&&fabsf(state.value-.3f)<.001f);
    mute_flags[0]=1;mute_writable=0;
    assert(rm_set_device_mute("fixture",0,&state)&&mute_flags[0]==1);
    assert(rm_set_device_volume("fixture",.5f,&state)&&mute_flags[0]==1&&fabsf(levels[0]-.3f)<.001f);
    mute_writable=1;
    master=0;mute_layout=2;mute_flags[1]=1;mute_flags[2]=0;
    assert(!rm_set_device_volume("fixture",.4f,&state)&&mute_flags[1]==1&&mute_flags[2]==0);
    mute_flags[2]=1;
    assert(!rm_set_device_volume("fixture",.6f,&state)&&mute_flags[1]==0&&mute_flags[2]==0);
    assert(!rm_set_device_mute("fixture",1,&state)&&state.muted&&fabsf(state.value-.6f)<.001f);
    assert(!rm_set_device_mute("fixture",0,&state)&&!state.muted&&fabsf(state.value-.6f)<.001f);
    puts("PASS: read-only discovery, absent/read-only controls, bounds, hardware readback, channel balance, failure rollback, explicit fixed maximum");
}
