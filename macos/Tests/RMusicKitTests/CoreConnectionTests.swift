import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class ForegroundSearchRuntime: RuntimeTransport {
    var searches = 0
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if body["path"].text.hasPrefix("/library/tracks?") {
            searches += 1
            return .object(["items": .array([]), "total": .number(0)])
        }
        return .object(["items": .array([])])
    }
    func shutdown() async {}
}

@Test @MainActor func changingWindowFocusKeepsTheScheduledLibrarySearch() async throws {
    let runtime = ForegroundSearchRuntime()
    let store = MusicStore(runtime: runtime)
    store.connected = true
    store.searchChanged()
    store.setUIActive(false)
    let deadline = ContinuousClock.now + .seconds(2)
    while runtime.searches == 0 && ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }
    #expect(runtime.searches == 1)
    await store.shutdown()
}

@Test @MainActor func localNetworkReadinessOnlyChecksLANHosts() {
    for host in ["192.168.50.10", "192.168.1.2", "172.16.4.2", "172.31.1.2", "169.254.2.3", "nas.local", "nas", "[fd00::1]", "[fe80::1]"] {
        #expect(LocalNetworkReadiness.needsCheck("http://\(host):18854"), "\(host)")
    }
    for host in ["localhost", "127.0.0.1", "[::1]", "172.32.1.2", "192.0.2.1", "ddgnas.synology.me", "[2001:db8::1]"] {
        #expect(!LocalNetworkReadiness.needsCheck("https://\(host)"), "\(host)")
    }
}

@Test func coreAddressesNormalizeWithoutAcceptingCredentialsOrInvalidPorts() {
    #expect(CoreConnectionAddress.normalize("  NAS.local:18854/music/  ") == "http://nas.local:18854/music")
    #expect(CoreConnectionAddress.normalize("HTTPS://CORE:443/music/") == "https://core/music")
    #expect(CoreConnectionAddress.normalize("http://[::1]:18864/") == "http://[::1]:18864")
    for input in ["", "file:///tmp/core", "http://user:secret@nas", "https://nas/?key=secret", "https://nas/#x", "http://nas:0", "http://nas:65536", "http://nas:-1", "http://"] {
        #expect(CoreConnectionAddress.normalize(input) == nil, "\(input)")
    }
}

@Test func targetsDistinguishAReplacedCoreAtTheSameAddressAndPreferVerifiedLocalAddress() {
    let identity = CoreIdentity(address: "http://127.0.0.1:18864", core_id: "new", name: "New Core", version: "1", initialized: true)
    let old = CoreProfile(name: "Old Core", address: identity.address, core_id: "old")
    let replaced = CoreConnectionTarget.choices(profile: old, local: identity, discovered: [])
    #expect(replaced.map(\.id) == ["old", "new"])
    #expect(replaced[0].saved && replaced[1].local)
    let same = CoreProfile(name: "My Core", address: "http://192.0.2.1:18854", core_id: "new")
    let choices = CoreConnectionTarget.choices(profile: same, local: identity, discovered: [])
    #expect(choices.count == 1 && choices[0].address == identity.address && choices[0].name == "My Core")
}

@MainActor private final class ConnectionPreviewRuntime: RuntimeTransport {
    var previews: [(JSONValue, CheckedContinuation<JSONValue, Never>)] = []
    var attempts: [JSONValue] = []
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "inspect_core" { return await withCheckedContinuation { previews.append((body, $0)) } }
        if operation == "connect" { attempts.append(body); throw RuntimeFailure(message: "Invalid login", status: 403) }
        return .null
    }
    func shutdown() async {}
}

@Test @MainActor func confirmedSwitchIsADraftAndFailedLoginRetainsTheLastSuccessfulCore() async {
    let runtime = ConnectionPreviewRuntime(), store = MusicStore(runtime: runtime)
    store.profile = CoreProfile(name: "Saved", address: "https://saved.example", core_id: "saved")
    store.lanAddress = "http://127.0.0.1:18864"; store.remoteAddress = "https://old-backup.example"
    store.username = "qa"; store.password = "old-password"; store.totp = "123456"
    store.error = "old error"
    let actual = CoreIdentity(address: store.lanAddress, core_id: "actual", name: "Actual", version: "1", initialized: true)
    store.adoptConnectionIdentity(actual)
    #expect(store.profile.core_id == "saved" && store.profile.address == "https://saved.example")
    #expect(store.connectionBinding.core_id == "actual" && store.username == "qa")
    #expect(store.password.isEmpty && store.totp.isEmpty && store.remoteAddress.isEmpty && store.error == nil)
    store.password = "wrong-password"
    await store.connect()
    #expect(runtime.attempts.count == 1 && runtime.attempts[0]["profile"]["core_id"].text == "actual")
    #expect(store.profile.core_id == "saved" && store.password == "wrong-password")
    #expect(store.lanAddress == actual.address && !store.connected && !store.connecting)
    #expect(store.error == "账号、密码或两步验证码不正确，请检查后重试。")
    store.lanAddress = "https://different.example"
    #expect(store.connectionBinding.core_id == "saved")
    await store.shutdown()
}

@Test @MainActor func previewDiscardsLateOrCancelledResponsesAndSendsNoAccountFields() async throws {
    let runtime = ConnectionPreviewRuntime(), store = MusicStore(runtime: runtime), inspector = CoreConnectionInspector()
    store.username = "private-user"; store.password = "private-password"
    func waitFor(_ count: Int) async throws {
        let deadline = ContinuousClock.now + .seconds(2)
        while runtime.previews.count < count, ContinuousClock.now < deadline { await Task.yield() }
        try #require(runtime.previews.count == count)
    }
    func response(_ address: String, _ id: String) -> JSONValue {
        .object(["address": .string(address), "core_id": .string(id), "name": .string(id), "version": .string("1"), "initialized": .bool(true)])
    }
    let first = Task { await inspector.inspect(CoreProfile(address: "http://first"), store: store, debounce: false) }
    try await waitFor(1)
    let second = Task { await inspector.inspect(CoreProfile(address: "http://second"), store: store, debounce: false) }
    try await waitFor(2)
    for (body, _) in runtime.previews {
        #expect(!String(decoding: try JSONEncoder().encode(body), as: UTF8.self).contains("private-"))
        #expect(body["alternatives"].array.isEmpty)
    }
    runtime.previews[1].1.resume(returning: response("http://second", "second")); await second.value
    runtime.previews[0].1.resume(returning: response("http://first", "first")); await first.value
    #expect(inspector.identity?.core_id == "second" && !inspector.checking)
    let third = Task { await inspector.inspect(CoreProfile(address: "http://third"), store: store, debounce: false) }
    try await waitFor(3); third.cancel(); inspector.clear()
    runtime.previews[2].1.resume(returning: response("http://third", "third")); await third.value
    #expect(inspector.identity == nil && !inspector.checking && inspector.error == nil)
    await store.shutdown()
}
