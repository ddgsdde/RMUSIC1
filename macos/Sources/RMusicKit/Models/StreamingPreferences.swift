import Foundation

struct StreamingPreferences: Codable, Equatable {
    var quality = "original"
    var mode = "auto"
    var max_sample_rate = 768_000
    var max_bit_depth = 32
    var buffer_seconds = 30
    init() {}
    enum CodingKeys: String, CodingKey { case quality, mode, max_sample_rate, max_bit_depth, buffer_seconds }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        mode = try c.decodeIfPresent(String.self, forKey: .mode) ?? "auto"
        max_sample_rate = try c.decodeIfPresent(Int.self, forKey: .max_sample_rate) ?? 768_000
        max_bit_depth = try c.decodeIfPresent(Int.self, forKey: .max_bit_depth) ?? 32
        buffer_seconds = try c.decodeIfPresent(Int.self, forKey: .buffer_seconds) ?? 30
        let oldDefault = mode == "auto" && max_sample_rate == 96_000 && max_bit_depth == 24 && buffer_seconds == 15
        quality = try c.decodeIfPresent(String.self, forKey: .quality) ?? (oldDefault ? "original" : "custom")
        if oldDefault && !c.contains(.quality) { buffer_seconds = 30 }
    }
    static let rates = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000, 705_600, 768_000]
    static func load() -> Self {
        guard let data = UserDefaults.standard.data(forKey: "streamingPolicy"), let value = try? JSONDecoder().decode(Self.self, from: data), ["original", "custom"].contains(value.quality), ["auto", "lan", "remote", "vpn"].contains(value.mode), Self.rates.contains(value.max_sample_rate), [16,24,32].contains(value.max_bit_depth), (5...60).contains(value.buffer_seconds) else { return Self() }
        return value
    }
}
extension MusicStore {
    func saveStreamingPolicy(_ value: StreamingPreferences) async throws {
        if connected { _ = try await runtimeCall("streaming_policy", try .encoded(value)) }
        UserDefaults.standard.set(try JSONEncoder().encode(value), forKey: "streamingPolicy")
        streamingPolicy = value
    }
}
