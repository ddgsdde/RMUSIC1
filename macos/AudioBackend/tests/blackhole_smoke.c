// Opt-in virtual output verification. This test never selects a physical/default device.
#include "../native/coreaudio.h"
#include <assert.h>
#include <stdio.h>
#include <unistd.h>
int main(void) {
    RMOutput *output=NULL;
    assert(rm_open("BlackHole2ch_UID",48000,2,0,0,100,&output)==0);
    assert(rm_rate(output)==48000);
    uint8_t silence[4800*8]={0};
    assert(rm_write(output,silence,4800)==4800);
    assert(rm_start(output)==0);rm_finish(output);
    RMProgress progress={0};
    for(int i=0;i<200;i++) {
        assert(rm_progress(output,&progress)==0);
        assert(progress.played<=4800);
        if(progress.played==4800) break;
        usleep(5000);
    }
    assert(progress.played==4800);assert(progress.device_ns>0);assert(progress.underrun==0);
    rm_close(output);
    puts("PASS: BlackHole HAL output, 4800 silent frames, device timestamp, exact drain; no physical DAC");
}
