import Foundation

struct CoreAddress: Codable { var address: String; var from_core: Bool? = nil; var certificate_sha256: String?; var skip_certificate_verification: Bool? = nil }
struct CoreProfile: Codable {
    var id = "primary"
    var name = "我的 Core"
    var address = ""
    var skip_certificate_verification: Bool? = nil
    var alternatives: [CoreAddress] = []
    var core_id: String?
    var core_public_key: String? = nil
}
struct AudioDevice: Decodable, Identifiable {
    let id: String
    let name: String
    let mix_rate: UInt32
    let channels: UInt16
    let `default`: Bool
}
struct Track: Codable, Identifiable {
    let id: String
    let title: String
    let artist: String
    let album: String
    let duration: Double
    let format: String
    let sample_rate: Int
    let bit_depth: Int
    var available: Bool? = nil
    var cover_url: String? = nil
    var album_artist: String? = nil
    var source: String? = nil
    var recording_id: String? = nil
    var provider_data: JSONValue? = nil
    var integrated_lufs: Double? = nil
    var true_peak_dbfs: Double? = nil
    var track_number: Int? = nil
    var created_at: Int? = nil
    private var observedAudio: JSONValue {
        guard source != "local", let value = provider_data?["source_audio"], value["verified"].boolean else { return .null }
        return value
    }
    var declaredQuality: String? {
        guard let hint = provider_data?["quality_hint"], let label = hint["label"].string, !label.isEmpty else { return nil }
        return label + " · 平台声明"
    }
    var formatLabel: String {
        if let format = observedAudio["format"].string { return format.uppercased() }
        if !format.isEmpty && !["unknown", "stream"].contains(format.lowercased()) { return format.uppercased() }
        return declaredQuality ?? ((source ?? "local") == "local" ? "格式未报告" : "在线音源")
    }
    var quality: String {
        let audio = observedAudio
        let format = audio["format"].string ?? self.format
        let rate = audio["format"].string != nil ? audio["sample_rate"].integer : sample_rate
        let bits = audio["format"].string != nil ? audio["bit_depth"].integer : bit_depth
        if ["dsf", "dff"].contains(format.lowercased()), rate > 0 {
            let family = rate % 44100 == 0 ? 44100 : 48000
            return "DSD\(rate / family) · \(format.uppercased()) · \((Double(rate) / 1_000_000).formatted()) MHz"
        }
        var fields = [format.uppercased()].filter { !$0.isEmpty && $0 != "UNKNOWN" }
        if bits > 0 { fields.append("\(bits) bit") }
        if rate > 0 { fields.append("\((Double(rate) / 1000).formatted()) kHz") }
        return fields.isEmpty ? (declaredQuality ?? ((source ?? "local") == "local" ? "格式未报告" : "在线音源")) : fields.joined(separator: " · ")
    }
}
struct Zone: Codable, Identifiable {
    var id: String
    var name: String
    var endpoint_id: String
    var status: String
    var track: Track?
    var position: Double
    var volume: Double
    var muted: Bool
    var playback_id: String?
    var queue_index: Int
    var queue: [Track]
    var queue_revision: Int? = nil
    var queue_unchanged: Bool? = nil
    var dsp: JSONValue? = nil
    var transitions: JSONValue? = nil
    var autoplay: JSONValue? = nil
    var `repeat`: String? = nil
    var shuffle: Bool? = nil
    var updated_at: JSONValue? = nil
}
struct TrackPage: Decodable { let items: [Track]; let total: Int }
struct ZonePage: Decodable { let items: [Zone] }
struct RuntimeResponse: Decodable, Sendable { let id: String; let result: JSONValue?; let error: String?; let status: Int? }
struct RuntimeCommand: Encodable { let id: String; let operation: String; let body: JSONValue }
struct RuntimeFailure: LocalizedError {
    let message: String
    var status: Int? = nil
    var errorDescription: String? { message }
}
extension String {
    var pathComponentEncoded: String { addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? self }
}
func timeLabel(_ seconds: Double) -> String {
    let value = Int(max(0, seconds.isFinite ? seconds : 0))
    return String(format: "%d:%02d", value / 60, value % 60)
}
