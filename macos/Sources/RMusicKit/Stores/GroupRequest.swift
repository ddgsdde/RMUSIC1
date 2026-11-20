import Foundation

/// Timeout retries must repeat the same request, rather than create another playback action.
struct GroupRequest: Codable {
  let path: String
  let body: JSONValue
  init(path: String, fields: [String: JSONValue]) {
    self.path = path
    body = .object(fields.merging(["request_id": .string(UUID().uuidString)]) { _, new in new })
  }
  var requestID: String { body["request_id"].text }
  var valid: Bool {
    guard UUID(uuidString: requestID) != nil else { return false }
    if path == "/sync/starts" { return true }
    let parts = path.split(separator: "/")
    return parts.count == 4 && parts[0] == "sync" && parts[1] == "starts"
      && UUID(uuidString: String(parts[2])) != nil
      && ["control", "cancel", "recovery", "rate"].contains(String(parts[3]))
  }
  static func eligible(_ endpoint: JSONValue) -> Bool {
    endpoint["online"].boolean
      && [
        "playback_instance", "scheduled_start_v1", "scheduled_queue_v1", "group_output_v2",
        "group_output_final_v2", "measured_listens_v2", "device_clock_v1", "programme_clock_v1",
      ].allSatisfy { endpoint["capabilities"][$0].boolean }
  }
}
