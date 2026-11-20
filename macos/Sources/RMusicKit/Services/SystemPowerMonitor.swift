import AppKit

/// A single, bounded transition worker; a fast wake must wait for the HAL close to finish.
@MainActor final class SystemPowerMonitor {
    private let center: NotificationCenter
    private var observers: [NSObjectProtocol] = []
    private var worker: Task<Void, Never>?
    private var pendingSleep = false
    private var desiredSleep = false
    private var appliedSleep = false
    private var stopped = false
    private let transition: @MainActor (Bool) async -> Void

    init(center: NotificationCenter = NSWorkspace.shared.notificationCenter,
         transition: @escaping @MainActor (Bool) async -> Void) {
        self.center = center
        self.transition = transition
        for (name, sleeping) in [(NSWorkspace.willSleepNotification, true), (NSWorkspace.didWakeNotification, false)] {
            observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.receive(sleeping) }
            })
        }
    }
    private func receive(_ sleeping: Bool) {
        guard !stopped else { return }
        if sleeping && !desiredSleep { pendingSleep = true }
        desiredSleep = sleeping
        guard worker == nil else { return }
        worker = Task { [weak self] in
            while let self, !Task.isCancelled {
                if pendingSleep {
                    pendingSleep = false; appliedSleep = true
                    await transition(true)
                } else if appliedSleep != desiredSleep {
                    appliedSleep = desiredSleep
                    await transition(appliedSleep)
                } else {
                    worker = nil
                    return
                }
            }
        }
    }
    func stop() {
        stopped = true
        observers.forEach(center.removeObserver)
        observers = []
        worker?.cancel(); worker = nil
    }
    isolated deinit {
        observers.forEach(center.removeObserver)
        worker?.cancel()
    }
}
