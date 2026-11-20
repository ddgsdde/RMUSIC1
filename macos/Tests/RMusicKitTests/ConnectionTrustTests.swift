import Foundation
import Testing
@testable import RMusicKit

@Test func certificateSkipDefaultsOffAndPersistsOnlyExplicitHTTPSAddresses() throws {
    let old = try JSONDecoder().decode(CoreProfile.self, from: Data(#"{"id":"primary","name":"QA","address":"https://core/music","alternatives":[]}"#.utf8))
    var trust = ConnectionTrustPreferences(profile: old)
    #expect(!trust.allowed(for: old.address))
    trust.set(true, for: "https://CORE:443/music/")
    trust.set(true, for: "http://backup/music")
    let configured = trust.applying(to: old, primary: "https://core/music", alternatives: ["https://backup/music", "http://backup/music"])
    #expect(configured.skip_certificate_verification == true)
    #expect(configured.alternatives.allSatisfy { $0.skip_certificate_verification == false })
    let restored = ConnectionTrustPreferences(profile: try JSONDecoder().decode(CoreProfile.self, from: JSONEncoder().encode(configured)))
    #expect(restored.allowed(for: "https://core/music/"))
    #expect(!restored.allowed(for: "https://core/other") && !restored.allowed(for: "https://backup/music"))
    trust.changed(from: "https://core/music", to: "https://new-core/music")
    #expect(!trust.allowed(for: "https://core/music") && !trust.allowed(for: "https://new-core/music"))
    trust.set(true, for: "https://core/a%2Fb")
    #expect(trust.allowed(for: "https://core/a%2Fb"))
    #expect(!trust.allowed(for: "https://core/a/b"))
    trust.set(true, for: "https://core/music%2F")
    #expect(!trust.allowed(for: "https://core/music"))
    for address in ["http://core", "https://user:pass@core", "https://core/?token=x", "https://core/#x"] { #expect(ConnectionTrustPreferences.key(address) == nil) }
}

@Test func primaryAndFallbackTrustExceptionsCanBeEnabledAndRevokedIndependently() {
    var trust = ConnectionTrustPreferences()
    trust.set(true, for: "https://backup/music")
    let profile = trust.applying(to: CoreProfile(core_id: "bound-core"), primary: "http://nas/music", alternatives: ["https://backup/music", "https://discovered/music"])
    #expect(profile.core_id == "bound-core" && profile.skip_certificate_verification == false)
    #expect(profile.alternatives[0].skip_certificate_verification == true)
    #expect(profile.alternatives[1].skip_certificate_verification == false)
    trust.set(false, for: "https://backup/music")
    #expect(!trust.allowed(for: "https://backup/music"))
}

@MainActor private final class TrustRuntime: RuntimeTransport {
    var profile: JSONValue = .null
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "connect" {
            profile = body["profile"]
            return .object(["core_id": .string("qa-core"), "address": profile["address"], "user": .object(["id": .string("qa")])])
        }
        return .object(["id": .string("qa"), "name": .string("QA"), "host_id": .string("qa"), "items": .array([]), "total": .number(0), "track_ids": .array([]), "jobs": .object([:]), "used_bytes": .number(0), "limit_bytes": .number(1024)])
    }
    func shutdown() async {}
}

@Test @MainActor func nativeLoginSendsIndependentTrustFlagsToTheSharedRuntime() async {
    let runtime = TrustRuntime(), store = MusicStore(runtime: runtime)
    store.profile = CoreProfile(); store.connectionTrust = ConnectionTrustPreferences()
    store.lanAddress = "http://nas/music"; store.remoteAddress = "https://backup/music"
    store.username = "synthetic"; store.password = "synthetic"
    store.connectionTrust.set(true, for: store.remoteAddress)
    await store.connect()
    #expect(store.connected)
    #expect(!runtime.profile["skip_certificate_verification"].boolean)
    #expect(runtime.profile["alternatives"].array.first?["skip_certificate_verification"].boolean == true)
    await store.shutdown()
}
