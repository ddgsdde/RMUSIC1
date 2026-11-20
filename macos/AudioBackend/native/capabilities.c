#include "coreaudio.h"
#include <CoreAudio/CoreAudio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
extern AudioDeviceID rm_find_device(const char *uid);
static AudioObjectPropertyAddress property(UInt32 selector, UInt32 scope) {
    return (AudioObjectPropertyAddress){selector,scope,kAudioObjectPropertyElementMain};
}
// Read driver tuples, rather than inventing all combinations of rate, precision and channels.
int32_t rm_capabilities(const char *uid, RMFormat *formats, uint32_t capacity, RMCapabilities *result) {
    memset(result,0,sizeof(*result)); AudioDeviceID device=rm_find_device(uid); if(!device)return -70001;
    AudioObjectPropertyAddress p=property(kAudioDevicePropertyNominalSampleRate,kAudioObjectPropertyScopeGlobal);
    Boolean writable=false; if(!AudioObjectIsPropertySettable(device,&p,&writable)) result->rate_writable=writable;
    p=property(kAudioDevicePropertyHogMode,kAudioObjectPropertyScopeGlobal);
    writable=false; if(!AudioObjectIsPropertySettable(device,&p,&writable))result->exclusive=writable;
    p=property(kAudioDevicePropertyStreams,kAudioObjectPropertyScopeOutput);
    UInt32 size=0; OSStatus status=AudioObjectGetPropertyDataSize(device,&p,0,NULL,&size);
    if(status||size>65536)return status?status:-70001;
    AudioStreamID *streams=calloc(1,size?size:1);if(!streams)return -70004;
    status=AudioObjectGetPropertyData(device,&p,0,NULL,&size,streams);
    if(!status)for(UInt32 i=0;i<size/sizeof(*streams);i++){
        AudioObjectPropertyAddress fp=property(kAudioStreamPropertyAvailablePhysicalFormats,kAudioObjectPropertyScopeGlobal);
        UInt32 bytes=0;if(AudioObjectGetPropertyDataSize(streams[i],&fp,0,NULL,&bytes)||bytes>1048576)continue;
        AudioStreamRangedDescription *items=calloc(1,bytes?bytes:1);if(!items){status=-70004;break;}
        if(!AudioObjectGetPropertyData(streams[i],&fp,0,NULL,&bytes,items))for(UInt32 j=0;j<bytes/sizeof(*items);j++){
            AudioStreamBasicDescription f=items[j].mFormat; AudioValueRange range=items[j].mSampleRateRange;
            if(f.mFormatID!=kAudioFormatLinearPCM||!isfinite(range.mMinimum)||!isfinite(range.mMaximum)||range.mMinimum<1||range.mMaximum<range.mMinimum)continue;
            if(result->count>=capacity){status=-70001;break;}
            formats[result->count++]=(RMFormat){range.mMinimum,range.mMaximum,f.mBitsPerChannel,f.mChannelsPerFrame,(f.mFormatFlags&kAudioFormatFlagIsFloat)!=0};
        }
        free(items);if(status)break;
    }
    free(streams);return status;
}
