import Foundation

/// One balanced activity lease for active native playback; stop/pause/owner teardown releases it.
@MainActor final class PlaybackActivity {
  static func hasActiveOutput(zones: [Zone], enabledEndpoints: Set<String>) -> Bool {
    zones.contains { enabledEndpoints.contains($0.endpoint_id) && $0.status == "playing" && $0.track != nil }
  }
  private var token: NSObjectProtocol?
  private let begin: () -> NSObjectProtocol
  private let end: (NSObjectProtocol) -> Void
  init(
    begin: @escaping () -> NSObjectProtocol = {
      ProcessInfo.processInfo.beginActivity(
        options: [.userInitiated, .latencyCritical], reason: "R-Music CoreAudio playback")
    }, end: @escaping (NSObjectProtocol) -> Void = { ProcessInfo.processInfo.endActivity($0) }
  ) {
    self.begin = begin
    self.end = end
  }
  func update(playing: Bool) {
    if playing {
      if token == nil { token = begin() }
    } else if let token {
      self.token = nil
      end(token)
    }
  }
  isolated deinit { if let token { end(token) } }
}
