import Foundation
import Testing
@testable import RMusicKit

@Test func colonFractionLyricsRemainVisibleAndUseRealTiming() {
    let lines = parseLyrics("[00:00:00]纯音乐\n[00:12:25]正文\n[00:13:125]下一句", translation: "[00:12.25]Translation")
    #expect(lines.map(\.text) == ["纯音乐", "正文", "下一句"])
    #expect(lines.map(\.time) == [0, 12.25, 13.125])
    #expect(lines[1].translation == "Translation")
}

@Test func fragmentedLargeRuntimeRepliesAndMultipleFramesAreDecodedOnce() throws {
    let reader = RuntimeResponseReader()
    let payload = String(repeating: "x", count: 1_048_576)
    let bytes = Data(("{\"id\":\"large\",\"result\":\"" + payload + "\"}\n{\"id\":\"small\",\"result\":true}\n").utf8)
    var results: [RuntimeResponse] = []
    for start in stride(from: 0, to: bytes.count, by: 4096) {
        results += try reader.read(bytes.subdata(in: start..<min(start+4096, bytes.count))).get()
    }
    #expect(results.map(\.id) == ["large", "small"])
    #expect(results[0].result?.string?.count == payload.count)
    #expect(try reader.read(Data("{\"id\":\"next\"}\n".utf8)).get().first?.id == "next")
    // Exercise complete frames followed by a surviving fragment repeatedly, including
    // after the large frame. Compaction must preserve exactly the pending suffix.
    for i in 0..<256 {
        let head = Data("{\"id\":\"\(i)\"}\n{\"id\":\"fragment".utf8)
        #expect(try reader.read(head).get().first?.id == String(i))
        #expect(try reader.read(Data("\"}\n".utf8)).get().first?.id == "fragment")
    }
    let oversized = RuntimeResponseReader()
    #expect(throws: RuntimeFailure.self) { try oversized.read(Data(repeating: 32, count: 8*1024*1024+1)).get() }
}

@Test func runtimeWireParsingKeepsBooleansNumbersNullsAndUnicodeDistinct() throws {
    let reply = try RuntimeResponseReader().read(Data(#"{"id":"types","result":{"boolean":true,"zero":0,"one":1,"fraction":1.25,"unicode":"封面🎵","null":null,"array":[false,2]}}"#.utf8) + Data([10])).get()
    let result = try #require(reply.first?.result)
    #expect(result["boolean"].boolean)
    #expect(result["zero"].number == 0 && result["one"].number == 1)
    #expect(result["fraction"].number == 1.25 && result["unicode"].string == "封面🎵")
    #expect(result["null"].string == nil && result["array"].array.count == 2)
    #expect(throws: DecodingError.self) { try result["one"].decode(Bool.self) }
    #expect(throws: DecodingError.self) { try result["boolean"].decode(Int.self) }
}

@Test func savedSourceParametersTakePriorityOverAnUnverifiedPlatformHint() throws {
    let track = try JSONDecoder().decode(Track.self, from: Data(#"{"id":"source","title":"Song","artist":"Artist","album":"Album","duration":5,"source":"provider","format":"unknown","sample_rate":0,"bit_depth":0,"provider_data":{"quality_hint":{"label":"FLAC 无损"},"source_audio":{"verified":true,"format":"flac","sample_rate":44100,"bit_depth":16}}}"#.utf8))
    #expect(track.quality == "FLAC · 16 bit · 44.1 kHz")
    #expect(track.formatLabel == "FLAC")
}

@MainActor private final class PagedPlaybackRuntime: RuntimeTransport {
    var paths: [String] = []
    var changes = false
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        let path = body["path"].text
        paths.append(path)
        let query = URLComponents(string:path)?.queryItems ?? []
        let offset = Int(query.first { $0.name == "offset" }?.value ?? "0") ?? 0
        let count = min(200, 205-offset)
        return .object(["total":.number(changes && offset>0 ? 204:205),"items":.array((offset..<offset+count).map { i in .object(["id":.string("track-\(i)"),"title":.string("Song"),"artist":.string("Artist"),"album":.string("Album"),"duration":.number(5),"format":.string("flac"),"sample_rate":.number(44100),"bit_depth":.number(16)]) })])
    }
    func shutdown() async {}
}
@Test @MainActor func playAllReadsTheWholeFilteredDirectoryAndRejectsChangingPages() async throws {
    let runtime=PagedPlaybackRuntime(), store=MusicStore(runtime: PagedPlaybackRuntime())
    let own=MusicStore(runtime:runtime)
    let ids=try await collectLibraryPlaybackIDs("/library/tracks?album_id=release&sort=release_new&offset=50&limit=50",store:own)
    #expect(ids.count == 205 && ids.last == "track-204")
    #expect(runtime.paths.count == 2)
    #expect(runtime.paths.allSatisfy { $0.contains("album_id=release") && $0.contains("sort=release_new") })
    runtime.changes=true
    await #expect(throws: RuntimeFailure.self) { try await collectLibraryPlaybackIDs("/library/tracks",store:own) }
    #expect(store.tracks.isEmpty)
}
