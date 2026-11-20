import Foundation

struct Playlist: Decodable, Identifiable {
    let id: String
    let name: String
    let track_ids: [String]
    let alias_track_ids: [String]?
    let cover_url: String?
    let smart_query: JSONValue?
}
struct PlaylistPage: Decodable { let items: [Playlist] }
struct CacheRange: Decodable { let start: UInt64; let end: UInt64 }
struct CachedTrack: Decodable, Identifiable {
    let id: String
    let track_id: String
    let title: String?
    let artist: String?
    let total_bytes: UInt64
    let ranges: [CacheRange]
    let complete: Bool
    let manual_download: Bool
    let format: String
    var received: UInt64 { ranges.reduce(0) { $0 + $1.end - $1.start } }
}
struct DownloadJob: Decodable {
    let state: String
    let error: String?
    let received: UInt64?
    let total: UInt64?
}
struct CachePage: Decodable {
    let items: [CachedTrack]
    let jobs: [String: DownloadJob]
    let used_bytes: UInt64
    let limit_bytes: UInt64
    var total: Int? = nil
    var manual_bytes: UInt64? = nil
    var exports: JSONValue? = nil
}
struct LyricWord: Equatable { let time: Double; let text: String; var duration: Double? = nil }
struct LyricLine: Identifiable, Equatable {
    let id: Int
    let time: Double
    let text: String
    var translation: String?
    var romanization: String? = nil
    var words: [LyricWord] = []
}
/// LRC offsets apply to the entire document; duplicate timestamps carry the translation.
func parseLyrics(_ text: String, translation: String? = nil) -> [LyricLine] {
    let stamp = try! NSRegularExpression(pattern: #"\[(\d+):(\d+(?:\.\d+|:\d{1,3})?)\]"#)
    let offsetPattern = try! NSRegularExpression(pattern: #"(?i)\[offset:([+-]?\d+)\]"#)
    let wordPattern = try! NSRegularExpression(pattern: #"<\d+:\d+(?:\.\d+)?>"#)
    let source = text as NSString
    let offsetMatch = offsetPattern.matches(in: text, range: NSRange(location: 0, length: source.length)).last
    let offset = offsetMatch.flatMap { Double(source.substring(with: $0.range(at: 1))) }.map { $0 / 1000 } ?? 0
    var pairs: [(Double, String, [LyricWord])] = []
    for raw in text.components(separatedBy: .newlines) {
        if Task.isCancelled { return [] }
        let line = raw as NSString
        let matches = stamp.matches(in: raw, range: NSRange(location: 0, length: line.length))
        let stripped = stamp.stringByReplacingMatches(in: raw, range: NSRange(location: 0, length: line.length), withTemplate: "")
        let body = wordPattern.stringByReplacingMatches(in: stripped, range: NSRange(location: 0, length: (stripped as NSString).length), withTemplate: "").trimmingCharacters(in: .whitespaces)
        for match in matches {
            if let minutes = Double(line.substring(with: match.range(at: 1))), let seconds = Double(line.substring(with: match.range(at: 2)).replacingOccurrences(of: ":", with: ".")), seconds < 60 {
                let time = minutes * 60 + seconds + offset
                pairs.append((time, body, matches.count == 1 ? lyricWords(stripped, offset: offset, lineTime: time) : []))
            }
        }
    }
    var result: [LyricLine] = []
    for (time, body, words) in pairs.sorted(by: { $0.0 < $1.0 }) {
        if result.last?.time == time { result[result.count - 1].translation = body }
        else { result.append(LyricLine(id: result.count, time: time, text: body, words: words)) }
    }
    if result.isEmpty {
        result = text.components(separatedBy: .newlines).filter { !$0.isEmpty && !$0.hasPrefix("[") }.enumerated().map { LyricLine(id: $0.offset, time: -1, text: $0.element) }
    }
    if let translation, !translation.isEmpty {
        let translated = parseLyrics(translation)
        var cursor = 0
        for index in result.indices {
            if result[index].time < 0, translated.indices.contains(index), translated[index].time < 0 { result[index].translation = translated[index].text }
            else if !translated.isEmpty {
                while cursor + 1 < translated.count && translated[cursor + 1].time <= result[index].time { cursor += 1 }
                let end = min(translated.count, cursor + 2)
                if let match = translated[cursor..<end].filter({ $0.time >= 0 && abs($0.time - result[index].time) <= 0.08 }).min(by: { abs($0.time - result[index].time) < abs($1.time - result[index].time) }) { result[index].translation = match.text }
            }
        }
    }
    for index in result.indices {
        if let value = result[index].translation?.trimmingCharacters(in: .whitespacesAndNewlines), value.isEmpty || value == "//" {
            result[index].translation = nil
        }
    }
    return result
}

struct AlbumGroup: Decodable { let id: String; let title: String; let artist: String }
struct AlbumPage: Decodable { let items: [AlbumGroup] }
