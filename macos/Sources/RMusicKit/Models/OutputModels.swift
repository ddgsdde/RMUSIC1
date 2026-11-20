import Foundation

struct OutputConfiguration: Decodable {
    let enabled: Bool
    let device_id: String?
    let exclusive: Bool
    let buffer_ms: Int
    let max_sample_rate: Int?
    let max_bit_depth: Int?
    var volume_control: String? = nil
}
struct DeviceVolumeState: Decodable, Equatable { let supported: Bool; let value: Double?; var muted: Bool? = nil; var mute_supported: Bool? = nil }
struct NativeOutput: Decodable, Identifiable {
    let id: String
    let uid: String
    let name: String
    let available: Bool
    let enabled: Bool
    let online: Bool
    let mix_rate: Int
    let channels: Int
    let system_default: Bool
    var route_name: String? = nil
    var followsSystemDefault: Bool { uid == "rmusic:system-default" }
    let config: OutputConfiguration
    let report: JSONValue?
    var capabilities: JSONValue? = nil
    var rates: [Int] { capabilities?["sample_rates"].array.map(\.integer) ?? [] }
    var bitDepths: [Int] { capabilities?["bit_depths"].array.map(\.integer) ?? [] }
    var device_volume: DeviceVolumeState? = nil
    var hardware_volume_enabled: Bool? = nil
    var error: String? { report?["error"]["error"].string ?? (report?["registered"].boolean == true ? nil : report?["connection_error"].string) }
    var status: String { !available ? "设备未连接" : !enabled ? "未启用" : online ? "已启用" : "连接音区中" }
}
struct OutputInventory: Decodable { let host_id: String; let items: [NativeOutput]; var removed: Bool?; var roaming: Bool? }
struct DiscoveredCore: Decodable, Identifiable {
    let id: String
    let name: String
    let addresses: [String]
    let core_id: String?
}
struct DiscoveryPage: Decodable { let items: [DiscoveredCore] }
