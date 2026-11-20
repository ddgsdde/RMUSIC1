import Foundation

extension JSONValue {
    var number: Double { if case .number(let value) = self { return value }; return 0 }
    var integer: Int { let value = number; return value.isFinite && value >= Double(Int.min) && value < Double(Int.max) ? Int(value) : 0 }
    var text: String { string ?? "" }
    var object: [String: JSONValue] { if case .object(let value) = self { return value }; return [:] }
}
func apiQuery(_ path: String, _ values: [String: String]) -> String {
    var parts = URLComponents()
    parts.queryItems = values.sorted { $0.key < $1.key }.map { URLQueryItem(name: $0.key, value: $0.value) }
    return path + "?" + (parts.percentEncodedQuery ?? "")
}
func listeningTime(_ seconds: Double) -> String {
    if seconds >= 3600 { return String(format: "%.1f 小时", seconds / 3600) }
    if seconds >= 60 { return "\(Int(seconds / 60)) 分钟" }
    return "\(Int(max(0, seconds))) 秒"
}
func displayDate(_ seconds: Double) -> String {
    seconds > 0 ? Date(timeIntervalSince1970: seconds).formatted(date: .abbreviated, time: .shortened) : "—"
}
