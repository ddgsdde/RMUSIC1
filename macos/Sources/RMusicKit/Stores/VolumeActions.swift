import Foundation

enum OutputVolumeMode: Equatable { case device, software, fixed }

func softwareVolumePatch(_ level: Double) -> JSONValue {
    var patch: [String: JSONValue] = ["volume": .number(level)]
    if level > 0 { patch["muted"] = .bool(false) }
    return .object(patch)
}

extension MusicStore {
    var volumeTargetID: String { "\(sessionID):\(zone?.id ?? ""):\(zone?.endpoint_id ?? ""):\(volumeMode)" }
    func acceptDeviceVolume(_ state: JSONValue, endpoint: String) {
        if let index = outputs.items.firstIndex(where: { $0.id == endpoint }),
           let actual = try? state.decode(DeviceVolumeState.self), actual != outputs.items[index].device_volume {
            outputs.items[index].device_volume = actual
        }
        guard signalEndpoint["id"].string == endpoint, case .object(var value) = signalEndpoint else { return }
        if value["device_volume"] != state { value["device_volume"] = state; signalEndpoint = .object(value) }
    }
    var volumeMode: OutputVolumeMode {
        guard let output = outputs.output(for: zone) else {
            return signalEndpoint["id"].string == zone?.endpoint_id ? (signalEndpoint["output_config"]["volume_control"].text == "software" ? .software : signalEndpoint["output_config"]["volume_control"].text == "device" ? .device : .fixed) : .fixed
        }
        if output.config.volume_control == "software" { return .software }
        return output.device_volume?.supported == true && output.config.volume_control == "device" ? .device : .fixed
    }
    var canMuteLocalDevice: Bool {
        guard let output = outputs.output(for: zone), output.enabled, output.available else { return false }
        return output.device_volume?.mute_supported == true
    }
    var deviceVolumeMuted: Bool? {
        if let output = outputs.output(for: zone) { return output.device_volume?.muted }
        if signalEndpoint["id"].string == zone?.endpoint_id, case .bool(let muted) = signalEndpoint["device_volume"]["muted"] {
            return muted
        }
        return nil
    }
    var volumeValue: Double {
        switch volumeMode {
        case .device:
            if let output = outputs.output(for: zone) { output.device_volume?.value ?? 1 }
            else if signalEndpoint["id"].string == zone?.endpoint_id, signalEndpoint["device_volume"]["supported"].boolean { signalEndpoint["device_volume"]["value"].number }
            else { outputs.output(for: zone)?.device_volume?.value ?? 1 }
        case .software: zone?.volume ?? 1
        case .fixed: 1
        }
    }
    var volumeLabel: String {
        switch volumeMode { case .device: "设备音量"; case .software: "软件音量"; case .fixed: "固定最大" }
    }
    func setVolumeMode(_ mode: OutputVolumeMode) async {
        guard let output = outputs.output(for: zone) else { return }
        await perform {
            try await requireUngrouped(zone?.id ?? "")
            try await outputs.configure(output, changes: [
                "volume_control": .string(mode == .software ? "software" : mode == .device ? "device" : "fixed"),
                "volume_max": .number(1),
                "hardware_volume_enabled": .bool(mode != .fixed)
            ], store: self)
        }
    }
}
