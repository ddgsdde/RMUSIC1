// Software callback checks only. No AudioUnit is opened and no physical device is touched.
#include <CoreAudio/CoreAudio.h>
static AudioTimeStamp fixture_clock;
static unsigned fixture_queries;
static OSStatus fixture_device_time(AudioObjectID device,AudioTimeStamp *stamp){(void)device;fixture_queries++;*stamp=fixture_clock;return noErr;}
#define AudioDeviceGetCurrentTime fixture_device_time
#include "../native/coreaudio.c"
#undef AudioDeviceGetCurrentTime
#include <assert.h>
#include <stdio.h>

int main(void) {
    RMOutput o={0}; o.rate=768000;o.channels=2;o.width=8;o.capacity=8;
    o.ring=calloc(o.capacity,o.width); assert(o.ring);
    int32_t samples[16];for(int i=0;i<16;i++)samples[i]=i%2?-i:i*100003;
    assert(rm_write(&o,(uint8_t*)samples,8)==8);
    assert(rm_write(&o,(uint8_t*)samples,1)==0);
    int32_t result[16]={0};
    AudioBufferList buffers={1,{{2,sizeof(result),result}}};
    AudioTimeStamp time={0};time.mFlags=kAudioTimeStampHostTimeValid|kAudioTimeStampSampleTimeValid;
    time.mHostTime=AudioConvertNanosToHostTime(AudioConvertHostTimeToNanos(AudioGetCurrentHostTime())-100000000);
    time.mSampleTime=1234;
    rm_pause(&o,1);assert(render(&o,NULL,&time,0,4,&buffers)==noErr);assert(atomic_load(&o.read)==0);
    for(int i=0;i<16;i++)assert(result[i]==0);
    rm_pause(&o,0);time.mHostTime+=AudioConvertNanosToHostTime(10000);time.mSampleTime+=4;assert(render(&o,NULL,&time,0,4,&buffers)==noErr);
    assert(memcmp(samples,result,4*8)==0);
    assert(rm_write(&o,(uint8_t*)samples,4)==4); // wrap the producer
    time.mSampleTime+=4;time.mHostTime+=AudioConvertNanosToHostTime(10000);
    assert(render(&o,NULL,&time,0,8,&buffers)==noErr);
    assert(memcmp(samples+8,result,4*8)==0);
    assert(memcmp(samples,result+8,4*8)==0);
    assert(atomic_load(&o.delivered)==12);
    RMProgress p;assert(rm_progress(&o,&p)==0);assert(p.played==12);assert(p.device_frame==1242);assert(p.queued==0&&p.capacity==8&&p.quantum==8);
    assert(!atomic_load(&o.underrun));rm_finish(&o);
    time.mHostTime+=AudioConvertNanosToHostTime(10000);time.mSampleTime+=8;assert(render(&o,NULL,&time,0,8,&buffers)==noErr);assert(!atomic_load(&o.underrun));
    atomic_store(&o.eof,0);time.mHostTime+=AudioConvertNanosToHostTime(10000);time.mSampleTime+=8;assert(render(&o,NULL,&time,0,8,&buffers)==noErr);assert(atomic_load(&o.underrun));
    RMFeedDiagnostic diagnostic;rm_feed_diagnostic(&o,&diagnostic);
    assert(diagnostic.queued==0&&diagnostic.requested==8);
    time.mFlags=0;assert(render(&o,NULL,&time,0,8,&buffers)==noErr);assert(rm_progress(&o,&p)==-70009);
    free(o.ring);
    RMOutput tail={0};tail.rate=48000;tail.channels=2;tail.width=8;tail.capacity=8;tail.ring=calloc(8,8);
    assert(rm_write_final(&tail,(uint8_t*)samples,3)==3);assert(atomic_load(&tail.eof));
    time.mFlags=kAudioTimeStampHostTimeValid|kAudioTimeStampSampleTimeValid;time.mSampleTime=0;
    assert(render(&tail,NULL,&time,0,8,&buffers)==noErr);assert(atomic_load(&tail.delivered)==3);assert(!atomic_load(&tail.underrun));
    assert(memcmp(samples,result,3*8)==0);free(tail.ring);
    // Start inside the next callback at a shared presentation deadline. Prefix/suffix are silence.
    RMOutput timed={0};timed.rate=48000;timed.channels=2;timed.width=8;timed.capacity=16;timed.ring=calloc(16,8);
    assert(rm_write_final(&timed,(uint8_t*)samples,3)==3);
    uint64_t first_ns=AudioConvertHostTimeToNanos(AudioGetCurrentHostTime())-20000000;
    time.mHostTime=AudioConvertNanosToHostTime(first_ns);first_ns=AudioConvertHostTimeToNanos(time.mHostTime);time.mSampleTime=2000;
    uint64_t deadline=first_ns+10ULL*1000000000/48000;
    atomic_store(&timed.start_presentation_ns,deadline);
    assert(render(&timed,NULL,&time,0,8,&buffers)==0);assert(atomic_load(&timed.delivered)==0);assert(!atomic_load(&timed.underrun));
    for(int i=0;i<16;i++)assert(result[i]==0);
    time.mHostTime=AudioConvertNanosToHostTime(first_ns+8ULL*1000000000/48000);time.mSampleTime+=8;
    assert(render(&timed,NULL,&time,0,8,&buffers)==0);assert(atomic_load(&timed.delivered)==3);assert(!atomic_load(&timed.underrun));
    Span sent=timed.spans[0];assert(sent.start_ns>=deadline&&sent.start_ns-deadline<1000000000/48000+100);
    uint32_t prefix=(uint32_t)(((sent.start_ns-AudioConvertHostTimeToNanos(time.mHostTime))*48000+500000000)/1000000000);
    assert(prefix>=2&&prefix<=3);for(uint32_t i=0;i<prefix*2;i++)assert(result[i]==0);assert(memcmp(samples,result+prefix*2,3*8)==0);
    timed.device=1;timed.scheduled_output=1;fixture_clock=time;fixture_clock.mHostTime=AudioConvertNanosToHostTime(sent.start_ns+45000);fixture_clock.mSampleTime=2020;
    assert(rm_progress(&timed,&p)==0);assert(p.capture_valid);assert(p.capture_frame==2020);assert(p.programme_frames==2);assert(p.capture_before<=p.capture_after);
    unsigned calls=fixture_queries;uint64_t captured_before=p.capture_before;assert(rm_progress(&timed,&p)==0);assert(fixture_queries==calls);assert(p.capture_before==captured_before);
    free(timed.ring);
    puts("PASS: PCM32 768kHz exact bytes, bounded ring/wrap, pause, timestamp-based final frames, underrun, EOF, invalid clock, frame-aligned scheduled prefix and paired device/programme capture");
    return 0;
}
