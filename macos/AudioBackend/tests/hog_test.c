// Synthetic HAL only; no real device is opened, claimed or modified.
#include <CoreAudio/CoreAudio.h>
#include <unistd.h>
#include <assert.h>
#include <stdio.h>
static pid_t owner=-1;static int writes,refuse,readback_failed;
static OSStatus fake_get(AudioObjectID id,const AudioObjectPropertyAddress *a,UInt32 q,const void *v,UInt32 *size,void *out){
    (void)id;(void)q;(void)v;(void)size;
    if(a->mSelector!=kAudioDevicePropertyHogMode)return -1;
    if(readback_failed&&writes)return -1;
    *(pid_t*)out=owner;return 0;
}
static OSStatus fake_set(AudioObjectID id,const AudioObjectPropertyAddress *a,UInt32 q,const void *v,UInt32 size,const void *in){
    (void)id;(void)q;(void)v;(void)size;(void)in;
    if(a->mSelector!=kAudioDevicePropertyHogMode)return -1;
    writes++;if(!refuse)owner=owner==getpid()?-1:getpid();return 0;
}
#define AudioObjectGetPropertyData fake_get
#define AudioObjectSetPropertyData fake_set
#include "../native/coreaudio.c"
int main(void){
    RMOutput output={0};output.device=123;
    assert(!acquire_hog(&output)&&output.hog&&owner==getpid());
    // A second session in this process cannot steal/toggle the first session's lease.
    RMOutput second={0};second.device=123;assert(acquire_hog(&second)==RM_BUSY&&writes==1);
    release_hog(&second);assert(owner==getpid()&&writes==1);
    release_hog(&output);assert(owner==-1&&writes==2);
    owner=12345;assert(acquire_hog(&output)==RM_BUSY&&writes==2);
    owner=-1;refuse=1;assert(acquire_hog(&output)==RM_BUSY&&!output.hog);
    puts("PASS: actual HAL readback, unchanged input pointer, same-process/foreign ownership, release and refused claim; synthetic only");
}
