import Foundation

/// Bounded visual metadata. No audio or credentials cross the renderer boundary.
struct MineradioPlayback: Equatable {
    let id: String
    let session: String
    let queueIndex: Int
    let title: String
    let artist: String
    let position: Double
    let duration: Double
    let playing: Bool
    var lyricOffset = 0.0
    var sampledAt = ProcessInfo.processInfo.systemUptime
    var dictionary: [String: Any] { ["id": id, "session": session, "queueIndex": queueIndex, "title": title, "artist": artist, "position": position + (playing ? max(0, ProcessInfo.processInfo.systemUptime - sampledAt) : 0), "duration": duration, "playing": playing, "lyricOffset": lyricOffset, "sentAtMS": Date.now.timeIntervalSince1970 * 1000] }
}
struct MineradioLyric: Equatable {
    let time: Double
    let text: String
    let translation: String?
    let words: [LyricWord]
    var dictionary: [String: Any] {
        var value: [String: Any] = ["time": time, "text": text]
        if !words.isEmpty { value["words"] = words.prefix(500).map { word in
            var item: [String: Any] = ["time": word.time, "text": word.text]
            if let duration = word.duration { item["duration"] = duration }
            return item
        } }
        if let translation { value["translation"] = translation }
        return value
    }
}
struct MineradioQueueItem: Equatable {
    let id: String; let title: String; let artist: String; let index: Int
    var cover = ""
    var dictionary: [String: Any] { ["id": id, "title": title, "artist": artist, "index": index, "cover": cover] }
}
