import Foundation

struct SmartPlaylistDraft: Equatable {
    var strings: [String: String] = [:]
    var favorites = false
    var excludeSaved = false
    static let numeric: [(String, String, Int)] = [("min_plays", "最少聆听次数", 1_000_000), ("max_plays", "最多聆听次数", 1_000_000), ("min_seconds", "最短时长（秒）", 86400), ("max_seconds", "最长时长（秒）", 86400), ("not_played_days", "至少多少天没听", 36500), ("min_sample_rate", "最低采样率（Hz）", 768000), ("min_bit_depth", "最低位深", 64), ("max_results", "最多选取", 10000)]
    init(_ value: JSONValue = .null) {
        for (field, item) in value.object { if let text = item.string { strings[field] = text } else if case .number = item { strings[field] = String(item.integer) } }
        favorites = value["favorites_only"].boolean; excludeSaved = value["exclude_saved"].boolean
        if strings["max_results"] == nil { strings["max_results"] = "100" }
    }
    func rule() throws -> JSONValue {
        var result: [String: JSONValue] = [:]
        for (key, value) in strings where !value.isEmpty { result[key] = .string(value) }
        for (key, title, max) in Self.numeric {
            if let value = strings[key], !value.isEmpty {
                guard let n = Int(value), n >= (key == "max_results" ? 1 : 0), n <= max else { throw RuntimeFailure(message: "\(title)须为有效整数，最大 \(max)。") }
                result[key] = .number(Double(n))
            }
        }
        for (min, max) in [("min_plays", "max_plays"), ("min_seconds", "max_seconds")] {
            if let lower = result[min]?.number, let upper = result[max]?.number, lower > upper { throw RuntimeFailure(message: "最小值不能超过最大值。") }
        }
        if favorites { result["favorites_only"] = .bool(true) }
        if excludeSaved { result["exclude_saved"] = .bool(true) }
        return .object(result)
    }
}
