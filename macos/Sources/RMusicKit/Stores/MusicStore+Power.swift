import Foundation

extension MusicStore {
    public func suspend() async {
        guard !systemSleeping else { return }
        travel.checkpoint(self)
        systemSleeping = true
        stopPolling()
        updateSystemPlayback()
        // Login is a serialized runtime operation. Queue the suspend behind it as well, so a
        // late successful login cannot reopen previously enabled outputs while the Mac sleeps.
        guard connected || connecting else { return }
        let owner = sessionID
        do {
            _ = try await runtimeCall("ui_visibility", .object(["visible": .bool(false)]))
            let inventory = try await runtimeCall("suspend_outputs").decode(OutputInventory.self)
            guard owner == sessionID else { return }
            outputs.accept(inventory)
        } catch { if owner == sessionID, connected { await handle(error) } }
    }

    public func wake() async {
        guard systemSleeping else { return }
        systemSleeping = false
        guard connected || connecting else { return }
        let owner = sessionID
        defer { if owner == sessionID { startPolling(); updateSystemPlayback() } }
        do {
            let inventory = try await runtimeCall("resume_outputs").decode(OutputInventory.self)
            try Task.checkCancellation()
            guard owner == sessionID else { return }
            outputs.accept(inventory)
            _ = try await runtimeCall("ui_visibility", .object(["visible": .bool(uiActive && !bridgeOnly)]))
            if connected { try await pollOnce(refreshOutputs: false) }
        } catch { if owner == sessionID { await handle(error) } }
    }
}
