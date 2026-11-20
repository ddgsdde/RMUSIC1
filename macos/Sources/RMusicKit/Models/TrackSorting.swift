import Foundation

let trackSortOptions = ["default", "title", "title_desc", "artist", "album", "added", "duration", "duration_desc"]
func trackOrder(_ tracks: [Track], by key: String) -> [Int] {
    guard key != "default" else { return Array(tracks.indices) }
    return tracks.enumerated().sorted { left, right in
        let a = left.element, b = right.element
        let comparison: ComparisonResult
        switch key {
        case "title", "title_desc": comparison = a.title.localizedStandardCompare(b.title)
        case "artist": comparison = a.artist.localizedStandardCompare(b.artist)
        case "album": comparison = "\(a.album_artist ?? a.artist)\u{0}\(a.album)".localizedStandardCompare("\(b.album_artist ?? b.artist)\u{0}\(b.album)")
        case "duration", "duration_desc": comparison = a.duration == b.duration ? .orderedSame : a.duration < b.duration ? .orderedAscending : .orderedDescending
        case "added": comparison = a.created_at == b.created_at ? .orderedSame : (a.created_at ?? 0) > (b.created_at ?? 0) ? .orderedAscending : .orderedDescending
        default: comparison = .orderedSame
        }
        if comparison == .orderedSame { return left.offset < right.offset }
        return ["title_desc", "duration_desc"].contains(key) ? comparison == .orderedDescending : comparison == .orderedAscending
    }.map(\.offset)
}
func sortedTracks(_ tracks: [Track], by key: String) -> [Track] { trackOrder(tracks, by: key).map { tracks[$0] } }
