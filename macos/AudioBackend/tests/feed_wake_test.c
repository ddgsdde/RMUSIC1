// Opt-in actual BlackHole HAL only. Background timer-coalescing regression.
#include "../native/coreaudio.c"
#include <assert.h>
#include <sys/resource.h>
#include <time.h>

static uint64_t now_ns(void) { return AudioConvertHostTimeToNanos(AudioGetCurrentHostTime()); }
int main(void) {
    RMOutput *o=NULL;
    int status=rm_open("BlackHole2ch_UID",48000,2,0,24,0,40,&o);
    if(status){fprintf(stderr,"BlackHole unavailable: %d\n",status);return 2;}
    int32_t samples[4800*2];for(unsigned i=0;i<4800*2;i++)samples[i]=(i%2?-1:1)*0x123400;
    rm_write(o,(const uint8_t*)samples,o->capacity<4800?o->capacity:4800);
    assert(rm_start(o)==0);
    assert(setpriority(PRIO_DARWIN_PROCESS,0,PRIO_DARWIN_BG)==0);
    uint64_t begin=now_ns(),maximum=0,loops=0;RMProgress p={0};
    while(now_ns()-begin<3000000000ULL) {
        uint64_t a=now_ns();
#ifdef RM_POLL_WAIT
        usleep(2000);
#else
        assert(rm_wait(o,10)==0);
#endif
        uint64_t elapsed=now_ns()-a;if(elapsed>maximum)maximum=elapsed;
        assert(rm_progress(o,&p)==0);
        rm_write(o,(const uint8_t*)samples,p.capacity-p.queued);loops++;
        rm_finish_cycle(o);
    }
    rm_pause(o,1);rm_progress(o,&p);
    fprintf(stderr,"HAL background wake: poll=%d capacity=%u max_wait_us=%llu loops=%llu underrun=%u\n",
#ifdef RM_POLL_WAIT
        1,
#else
        0,
#endif
        p.capacity,(unsigned long long)(maximum/1000),(unsigned long long)loops,p.underrun);
    rm_close(o);return p.underrun?1:0;
}
