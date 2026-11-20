import Foundation

func renderLyricDocument(_ document: JSONValue, language: String) throws -> [LyricLine] {
  let translations = document["translations"].array
  let bytes =
    document["text"].text.utf8.count + document["translation"].text.utf8.count
    + document["romanization"].text.utf8.count
    + translations.reduce(0) { $0 + $1["text"].text.utf8.count }
  guard bytes <= 1_048_576, translations.count <= 32 else {
    throw RuntimeFailure(message: "歌词内容过大，已停止解析。")
  }
  let translation =
    translations.first { $0["id"].string == language }?["text"].string ?? translations.first?[
      "text"
    ].string ?? document["translation"].string
  var lines = parseLyrics(document["text"].text, translation: translation)
  // QRC keeps the platform's real start AND duration. Attach only to identical
  // text at the same line boundary, including when a cached file supplies LRC.
  let timing = document["word_timing"].array
  guard timing.count <= 2000 else { throw RuntimeFailure(message: "逐字歌词内容过大，已停止解析。") }
  var timed: [(Double, String, [LyricWord])] = []
  var timingBytes = bytes, wordCount = 0
  for entry in timing {
    guard case .number(let start) = entry["time"] else { continue }
    let text = entry["text"].text
    let input = entry["words"].array
    timingBytes += text.utf8.count + input.reduce(0) { $0 + $1["text"].text.utf8.count }
    wordCount += input.count
    guard timingBytes <= 1_048_576, wordCount <= 20_000 else { throw RuntimeFailure(message: "逐字歌词内容过大，已停止解析。") }
    guard start.isFinite, start >= 0, start <= 86400, input.count <= 500 else { continue }
    var words: [LyricWord] = [], previous = start
    for value in input {
      guard case .number(let time) = value["time"], case .number(let duration) = value["duration"],
        time.isFinite, duration.isFinite, time >= previous, duration >= 0, time + duration <= 86400,
        !value["text"].text.isEmpty else { words = []; break }
      words.append(LyricWord(time: time, text: value["text"].text, duration: duration)); previous = time
    }
    if !words.isEmpty, words.map(\.text).joined() == text { timed.append((start, text, words)) }
  }
  for index in lines.indices {
    guard let match = timed.first(where: { abs($0.0 - lines[index].time) <= 0.08 && $0.1.trimmingCharacters(in: .whitespacesAndNewlines) == lines[index].text }) else { continue }
    var words = match.2
    words[0] = LyricWord(time: words[0].time, text: words[0].text.replacingOccurrences(of: #"^\s+"#, with: "", options: .regularExpression), duration: words[0].duration)
    let last = words.count - 1
    words[last] = LyricWord(time: words[last].time, text: words[last].text.replacingOccurrences(of: #"\s+$"#, with: "", options: .regularExpression), duration: words[last].duration)
    if words.map(\.text).joined() == lines[index].text { lines[index].words = words }
  }
  if let romanization = document["romanization"].string {
    let pronounced = parseLyrics(document["text"].text, translation: romanization)
    for index in lines.indices where pronounced.indices.contains(index) {
      lines[index].romanization = pronounced[index].translation
    }
  }
  return lines
}
