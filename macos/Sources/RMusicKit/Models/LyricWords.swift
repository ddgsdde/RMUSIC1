import Foundation

/// Enhanced LRC has real word starts, never inferred from line length or character count.
func lyricWords(_ raw: String, offset: Double, lineTime: Double) -> [LyricWord] {
  let expression = try! NSRegularExpression(pattern: #"<(\d+):(\d+(?:\.\d+)?)>"#)
  let source = raw as NSString
  let matches = expression.matches(
    in: raw, range: NSRange(location: 0, length: (raw as NSString).length))
  guard !matches.isEmpty, matches[0].range.location == 0, matches.count <= 8192 else { return [] }
  var words: [LyricWord] = []
  var previous = lineTime
  for (index, match) in matches.enumerated() {
    guard !Task.isCancelled, let minutes = Double(source.substring(with: match.range(at: 1))),
      let seconds = Double(source.substring(with: match.range(at: 2))), seconds < 60
    else { return [] }
    let time = minutes * 60 + seconds + offset
    guard time.isFinite, time >= previous else { return [] }
    previous = time
    let start = NSMaxRange(match.range)
    let end = index + 1 < matches.count ? matches[index + 1].range.location : source.length
    let text = source.substring(with: NSRange(location: start, length: end - start))
    if !text.isEmpty { words.append(LyricWord(time: time, text: text)) }
  }
  return words
}
