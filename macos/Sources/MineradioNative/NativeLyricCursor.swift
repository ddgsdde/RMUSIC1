import Foundation

/// A tiny clock correction at a line boundary must not reverse a running transition.
/// Session changes, seeking and paused positioning remain authoritative.
struct NativeLyricCursor {
    private var index = -1
    private var position: Double?
    mutating func reset() { self = Self() }
    mutating func select(lines: [NativeLyricLine], position next: Double, playing: Bool) -> Int {
        var lower = 0, upper = lines.count
        while lower < upper {
            let mid = (lower + upper) / 2
            if lines[mid].time <= next { lower = mid + 1 } else { upper = mid }
        }
        let candidate = lower - 1
        // Only keep an already-entered line within 80 ms of its boundary. This
        // cannot accumulate drift, and a larger correction/seek takes effect now.
        let boundaryJitter = playing && candidate < index && lines.indices.contains(index)
            && next >= lines[index].time - 0.08
            && position.map { next >= $0 - 0.08 } == true
        if !boundaryJitter { index = candidate }
        position = next
        return index
    }
}
