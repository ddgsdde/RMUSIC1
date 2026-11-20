import Foundation
import Testing
@testable import RMusicKit
private func catalogTrack(_ source: String? = "qqmusic", data: JSONValue = .object(["artist_mids": .array([.string("crush"), .string("iu")])])) -> Track {
    Track(id: "qqmusic:song", title: "Song", artist: "Crush / IU", album: "Album", duration: 100, format: "", sample_rate: 0, bit_depth: 0, source: source, provider_data: data)
}
@MainActor private final class CatalogRuntime: RuntimeTransport {
    var payload: JSONValue = .null
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        payload = body
        return .object(["resolved": .bool(true), "artists": .array([.object(["id": .string("a"), "name": .string("Crush")]), .object(["id": .string("b"), "name": .string("IU")])])])
    }
    func shutdown() async {}
}
@Test func catalogCreditsRequireProviderEvidence() {
    #expect(catalogTrack().creditNames == ["Crush", "IU"])
    let legacy: JSONValue = .object(["provider_metadata": .object(["matched": .bool(true), "source": .string("qqmusic"), "track": .object(["artist": .string("Crush / IU")]), "artists": .array([.object(["mid": .string("crush"), "name": .string("Crush")]), .object(["mid": .string("iu"), "name": .string("IU")])])])])
    #expect(catalogTrack(data: legacy).creditNames == ["Crush", "IU"])
    #expect(catalogTrack("local", data: .object([:])).creditNames == ["Crush / IU"])
}
@Test @MainActor func catalogSummaryRetainsAllResolvedPeopleAndSendsTrackIdentity() async throws {
    let runtime = CatalogRuntime(), store = MusicStore(runtime: runtime)
    let links = try await store.catalogReferences(catalogTrack(nil, data: .null), album: false, artistIndex: 0)
    #expect(links.map(\.name) == ["Crush", "IU"])
    #expect(runtime.payload["json"]["track_id"].text == "qqmusic:song")
    let individual = try await store.catalogReferences(catalogTrack(), album: false, artistIndex: 1)
    #expect(individual.map(\.name) == ["IU"])
    await store.shutdown()
}
