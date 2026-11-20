import Foundation

/// Only this output and this Core playback instance may surface a recovery action.
struct PlaybackRecovery {
  enum Kind { case buffering, connection, source, device, failure, continuity }
  let kind: Kind
  let message: String
  let recoverable: Bool
  static func resolve(zone: Zone?, report: JSONValue) -> Self? {
    guard let zone else { return nil }
    let playback = report["playback"]
    let error = report["error"]
    let errorMatches =
      error["zone_id"].string == zone.id && error["track_id"].string == zone.track?.id
      && (zone.playback_id == nil || error["core_playback_id"].string == zone.playback_id)
    if errorMatches, let message = error["error"].string {
      if error["automatic_recovery"].boolean {
        return Self(kind: .buffering, message: "播放短暂中断，正在自动恢复", recoverable: false)
      }
      return Self(
        kind: error["device_failed"].boolean
          ? .device : error["source_failed"].boolean ? .source : .failure, message: message,
        recoverable: error["recoverable"].boolean)
    }
    if let message = report["connection_error"].string {
      return Self(kind: .connection, message: message, recoverable: false)
    }
    if playback["core_playback_id"].string == zone.playback_id,
      playback["track_id"].string == zone.track?.id,
      playback["signal_path"]["processing"].array.contains(where: {
        $0.text.hasPrefix("Playback buffering:")
      })
    {
      return Self(kind: .buffering, message: "音源数据加载中，正在缓冲", recoverable: false)
    }
    if playback["core_playback_id"].string == zone.playback_id,
      playback["track_id"].string == zone.track?.id,
      playback["signal_path"]["processing"].array.contains(where: {
        $0.text.contains("CoreAudio underrun")
      })
    {
      // The callback proves a gap, not whether the network, decoder or scheduling
      // caused it. Do not diagnose a device fault from this historical flag.
      return Self(kind: .continuity, message: "音频输出曾短暂中断，已继续播放", recoverable: false)
    }
    return nil
  }
}
