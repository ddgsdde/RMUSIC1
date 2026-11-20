// Scope the deadline policy to HAL waits and ready-PCM copies. Metadata,
// timeline reports, teardown and network decoding stay outside this scope.
#pragma once
#include <AudioUnit/AudioUnit.h>
#include <CoreAudio/HostTime.h>
#include <mach/mach.h>
#include <mach/thread_policy.h>
#include <os/workgroup.h>
#include <pthread.h>
#include <pthread/qos.h>

typedef struct {
    thread_t thread;
    qos_class_t qos;
    int relative;
    thread_extended_policy_data_t previous;
    os_workgroup_t group;
    os_workgroup_join_token_s token;
    int joined, active;
} RMFeedPriority;
static int rm_feed_priority_init(RMFeedPriority *p,AudioUnit unit) {
    p->thread=mach_thread_self();
    pthread_get_qos_class_np(pthread_self(),&p->qos,&p->relative);
    mach_msg_type_number_t count=THREAD_EXTENDED_POLICY_COUNT;boolean_t is_default=FALSE;
    int status=thread_policy_get(p->thread,THREAD_EXTENDED_POLICY,(thread_policy_t)&p->previous,&count,&is_default);
    if(status)return status;
    UInt32 size=sizeof(p->group);
    // Workgroup lookup may be unsupported by a driver; the scoped deadline
    // policy still applies. A successful lookup returns a retained reference.
    if(AudioUnitGetProperty(unit,kAudioOutputUnitProperty_OSWorkgroup,kAudioUnitScope_Global,0,&p->group,&size)==noErr && p->group)
        p->joined=os_workgroup_join(p->group,&p->token)==0;
    return 0;
}
static int rm_feed_priority_begin(RMFeedPriority *p,uint32_t frames,uint32_t rate) {
    if(p->active)return 0;
    uint64_t period=(uint64_t)(frames?frames:512)*1000000000/rate;
    if(period<1000000)period=1000000;
    uint64_t constraint=period/4;if(constraint>2000000)constraint=2000000;
    thread_time_constraint_policy_data_t policy={
        (uint32_t)AudioConvertNanosToHostTime(period),
        (uint32_t)AudioConvertNanosToHostTime(constraint/2),
        (uint32_t)AudioConvertNanosToHostTime(constraint),TRUE};
    int status=thread_policy_set(p->thread,THREAD_TIME_CONSTRAINT_POLICY,(thread_policy_t)&policy,THREAD_TIME_CONSTRAINT_POLICY_COUNT);
    p->active=status==0;return status;
}
static void rm_feed_priority_end(RMFeedPriority *p) {
    if(!p->active)return;
    thread_policy_set(p->thread,THREAD_EXTENDED_POLICY,(thread_policy_t)&p->previous,THREAD_EXTENDED_POLICY_COUNT);
    if(p->qos!=QOS_CLASS_UNSPECIFIED)pthread_set_qos_class_self_np(p->qos,p->relative);
    p->active=0;
}
static void rm_feed_priority_close(RMFeedPriority *p) {
    rm_feed_priority_end(p);
    if(p->joined) { os_workgroup_leave(p->group,&p->token);p->joined=0; }
    if(p->group) { os_release(p->group);p->group=NULL; }
    if(p->thread) { mach_port_deallocate(mach_task_self(),p->thread);p->thread=0; }
}
