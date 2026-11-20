import Foundation
import Testing
@testable import RMusicKit

@Test func platformQRCOnlyAttachesToMatchingLyricsAndPreservesRealDurations() throws {
    let input = #"{"text":"[00:01.00]Hello world\n[00:08.00]其他句","word_timing":[{"time":1,"text":"Hello world","words":[{"time":1.2,"duration":0.18,"text":"Hello "},{"time":3.4,"duration":0.7,"text":"world"}]},{"time":8,"text":"不同版本","words":[{"time":8,"duration":2,"text":"不同版本"}]}]}"#
    let document=try JSONDecoder().decode(JSONValue.self,from:Data(input.utf8))
    let lines=try renderLyricDocument(document,language:"")
    #expect(lines[0].words.map(\.text).joined()=="Hello world")
    #expect(lines[0].words.map(\.duration)==[0.18,0.7])
    #expect(lines[1].words.isEmpty)
    #expect(try renderLyricDocument(.object(["text":.string("[00:01.00]Hello world")]),language:"").first?.words.isEmpty == true)
}
