import Foundation

/// Only source word timings may move the highlight edge. Ordinary LRC and
/// translations light as a whole line; word starts without durations are steps.
func nativeLyricProgress(line: NativeLyricLine, position: Double, stops: [Float], translation: Bool) -> Float {
    guard position >= line.time else { return 0 }
    guard !translation, !line.words.isEmpty else { return 1 }
    guard let index = line.words.lastIndex(where: { $0.time <= position }) else { return 0 }
    let word = line.words[index], prefix = line.wordOffsets[index]
    let fraction: Float
    if let duration = word.duration, duration.isFinite, duration > 0 {
        fraction = Float(max(0,min(1,(position-word.time)/duration)))
    } else { fraction = 1 }
    if !stops.isEmpty {
        let from = stops[min(stops.count-1,prefix)], to = stops[min(stops.count-1,prefix+word.text.utf16.count)]
        return from+(to-from)*fraction
    }
    return (Float(prefix)+Float(word.text.utf16.count)*fraction)/Float(max(1,line.text.utf16.count))
}
