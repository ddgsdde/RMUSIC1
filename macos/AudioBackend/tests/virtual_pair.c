// Ephemeral public aggregate containing ONLY BlackHole; never a physical DAC or default route.
#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>
#include <unistd.h>
#include <string.h>
#include <ctype.h>
int main(int argc, char **argv) {
    char uid[128];snprintf(uid,sizeof(uid),"com.rmusic.qa.blackhole.%d",getpid());
    if(argc!=1) {
        const char *prefix="com.rmusic.qa.blackhole.";
        if(argc!=3||strcmp(argv[1],"--uid")||strncmp(argv[2],prefix,strlen(prefix))||strlen(argv[2])<=strlen(prefix)||strlen(argv[2])>=sizeof(uid))return 1;
        for(const char *p=argv[2];*p;p++)if(!isalnum((unsigned char)*p)&&*p!='.'&&*p!='-')return 1;
        snprintf(uid,sizeof(uid),"%s",argv[2]);
    }
    CFStringRef blackhole=CFSTR("BlackHole2ch_UID");AudioDeviceID source=0,device=0,before=0,after=0;UInt32 size=sizeof(source);
    AudioObjectPropertyAddress translate={kAudioHardwarePropertyTranslateUIDToDevice,kAudioObjectPropertyScopeGlobal,kAudioObjectPropertyElementMain};
    if(AudioObjectGetPropertyData(kAudioObjectSystemObject,&translate,sizeof(blackhole),&blackhole,&size,&source)||!source)return 2;
    AudioObjectPropertyAddress default_device={kAudioHardwarePropertyDefaultOutputDevice,kAudioObjectPropertyScopeGlobal,kAudioObjectPropertyElementMain};
    size=sizeof(before);if(AudioObjectGetPropertyData(kAudioObjectSystemObject,&default_device,0,NULL,&size,&before))return 3;
    CFStringRef identity=CFStringCreateWithCString(NULL,uid,kCFStringEncodingUTF8);
    const void *subkeys[]={CFSTR(kAudioSubDeviceUIDKey)},*subvalues[]={blackhole};
    CFDictionaryRef sub=CFDictionaryCreate(NULL,subkeys,subvalues,1,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
    const void *subitems[]={sub};CFArrayRef subs=CFArrayCreate(NULL,subitems,1,&kCFTypeArrayCallBacks);
    int zero=0;CFNumberRef flag=CFNumberCreate(NULL,kCFNumberIntType,&zero);
    const void *keys[]={CFSTR(kAudioAggregateDeviceUIDKey),CFSTR(kAudioAggregateDeviceNameKey),CFSTR(kAudioAggregateDeviceSubDeviceListKey),CFSTR(kAudioAggregateDeviceMainSubDeviceKey),CFSTR(kAudioAggregateDeviceIsPrivateKey)};
    const void *values[]={identity,CFSTR("R-Music QA · BlackHole Pair"),subs,blackhole,flag};
    CFDictionaryRef description=CFDictionaryCreate(NULL,keys,values,5,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
    OSStatus status=AudioHardwareCreateAggregateDevice(description,&device);
    CFRelease(description);CFRelease(flag);CFRelease(subs);CFRelease(sub);CFRelease(identity);
    if(status||!device){fprintf(stderr,"aggregate create OSStatus %d\n",(int)status);return 4;}
    size=sizeof(after);AudioObjectGetPropertyData(kAudioObjectSystemObject,&default_device,0,NULL,&size,&after);
    if(before!=after){AudioHardwareDestroyAggregateDevice(device);fprintf(stderr,"default route changed unexpectedly\n");return 5;}
    printf("{\"uid\":\"%s\",\"device\":%u,\"default_unchanged\":true}\n",uid,device);fflush(stdout);
    while(getchar()!=EOF){}
    status=AudioHardwareDestroyAggregateDevice(device);fprintf(stderr,"owned QA aggregate destroyed: %d\n",(int)status);return status?6:0;
}
