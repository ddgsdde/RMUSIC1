import Foundation
import Testing
@testable import RMusicKit

private struct TreePayload: Codable, Equatable {
    enum Choice: String, Codable { case one, two }
    let number: Int
    let flag: Bool
    let choice: Choice
    let nested: [[String: [Int?]]]
    let missing: String?
    let url: URL
    let date: Date
    let data: Data
}
@Test func directTreeDecodingMatchesTheWireDecoderForNestedValues() throws {
    let expected=TreePayload(number:42,flag:true,choice:.two,nested:[["values":[1,nil,-2]]],missing:nil,url:URL(string:"https://example.test/music")!,date:Date(timeIntervalSinceReferenceDate:12.5),data:Data([0,255,42]))
    let wire=try JSONEncoder().encode(expected)
    let value=try JSONDecoder().decode(JSONValue.self,from:wire)
    #expect(try value.decode(TreePayload.self) == JSONDecoder().decode(TreePayload.self,from:wire))
    #expect(try value.decode([String:JSONValue].self)["number"]?.integer == 42)
}
@Test func directTreeDecodingRetainsLargeMetadataAndRejectsInvalidScalarTypes() throws {
    let input:JSONValue = .object(["id":.string("song"),"title":.string("Song"),"artist":.string("Artist"),"album":.string("Album"),"duration":.number(1),"format":.string("flac"),"sample_rate":.number(44100),"bit_depth":.number(16),"provider_data":.object(["description":.string(String(repeating:"detail",count:100000))])])
    let tracks=try JSONValue.array(Array(repeating:input,count:100)).decode([Track].self)
    #expect(tracks.count == 100 && tracks.last?.provider_data?["description"].string?.count == 600000)
    #expect(throws: DecodingError.self) { try JSONValue.number(1.5).decode(Int.self) }
    #expect(throws: DecodingError.self) { try JSONValue.number(-1).decode(UInt.self) }
    #expect(throws: DecodingError.self) { try JSONValue.string("true").decode(Bool.self) }
    #expect(throws: DecodingError.self) { try JSONValue.number(.infinity).decode(Double.self) }
}
