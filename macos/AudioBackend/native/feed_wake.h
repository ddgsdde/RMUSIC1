// A binary, allocation-free callback notification. The output worker owns its
// lifetime; HAL is stopped before the semaphore is destroyed.
#pragma once
#include <mach/mach.h>
#include <stdatomic.h>

typedef struct { semaphore_t semaphore; _Atomic uint32_t armed; } RMFeedWake;
static int rm_feed_wake_create(RMFeedWake *wake) {
    return semaphore_create(mach_task_self(),&wake->semaphore,SYNC_POLICY_FIFO,0);
}
static void rm_feed_wake_signal(RMFeedWake *wake) {
    // No token accumulation while the worker is busy, paused or shutting down.
    if(wake->semaphore && atomic_exchange_explicit(&wake->armed,0,memory_order_acq_rel))
        semaphore_signal(wake->semaphore);
}
static int rm_feed_wake_wait(RMFeedWake *wake,uint32_t milliseconds) {
    atomic_store_explicit(&wake->armed,1,memory_order_release);
    mach_timespec_t timeout={milliseconds/1000,(milliseconds%1000)*1000000};
    int status=semaphore_timedwait(wake->semaphore,timeout);
    atomic_store_explicit(&wake->armed,0,memory_order_release);
    return status==KERN_OPERATION_TIMED_OUT || status==KERN_ABORTED ? KERN_SUCCESS : status;
}
static void rm_feed_wake_close(RMFeedWake *wake) {
    atomic_store_explicit(&wake->armed,0,memory_order_release);
    if(wake->semaphore) { semaphore_destroy(mach_task_self(),wake->semaphore);wake->semaphore=0; }
}
