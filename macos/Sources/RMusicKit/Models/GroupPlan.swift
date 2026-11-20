import Foundation

struct GroupPlan {
  var queue: [Track] = []
  var monitor = true
  var errorMicroseconds = 1000
  var warmupMilliseconds = 3000
  var rateEnabled = false
  var automaticRate = false
  static func pcm(_ track: Track) -> Bool {
    track.available != false && track.sample_rate > 0 && track.sample_rate <= 768000
      && track.bit_depth > 1 && !["dsf", "dff", "unknown", ""].contains(track.format.lowercased())
  }
  func tracks(current: Track?) -> [Track] { queue.isEmpty ? current.map { [$0] } ?? [] : queue }
  func valid(current: Track?) -> Bool {
    let items = tracks(current: current)
    return !items.isEmpty && items.count <= 10000 && items.allSatisfy(Self.pcm)
      && (!monitor
        || (100...50000).contains(errorMicroseconds) && (1000...10000).contains(warmupMilliseconds))
      && (!automaticRate || rateEnabled)
  }
  func fields(rooms: Set<String>, current: Track?) throws -> [String: JSONValue] {
    guard (2...4).contains(rooms.count), valid(current: current),
      let first = tracks(current: current).first
    else { throw RuntimeFailure(message: "请选择二至四个可用音区与已确认格式的 PCM 歌曲。") }
    var fields: [String: JSONValue] = [
      "zone_ids": .array(rooms.sorted().map(JSONValue.string)), "track_id": .string(first.id),
      "position": .number(0), "queue": .array(tracks(current: current).map { .string($0.id) }),
      "measured_version": .number(2),
    ]
    if monitor {
      fields["sync_policy"] = .object([
        "max_known_error_us": .number(Double(errorMicroseconds)),
        "warmup_ms": .number(Double(warmupMilliseconds)),
      ])
    }
    if rateEnabled { fields["pcm_rate_adjustment"] = .bool(true) }
    if automaticRate { fields["automatic_rate_adjustment"] = .bool(true) }
    return fields
  }
  static func rateSupported(_ endpoint: JSONValue, automatic: Bool) -> Bool {
    eligibleKeys(automatic: automatic).allSatisfy { endpoint["capabilities"][$0].boolean }
  }
  private static func eligibleKeys(automatic: Bool) -> [String] {
    ["pcm_rate_adjustment_v1", "programme_clock_v2"] + (automatic ? ["group_rate_lease_v1"] : [])
  }
  static func seek(from: Int, to: Int, position: Double) -> [String: JSONValue] {
    var fields: [String: JSONValue] = [
      "action": .string(from == to ? "seek" : "jump"), "queue_index": .number(Double(to)),
      "position": .number(position),
    ]
    if from != to { fields["from_queue_index"] = .number(Double(from)) }
    return fields
  }
}
