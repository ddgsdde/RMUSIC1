import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class LoginPresentationRuntime: RuntimeTransport {
    var library: CheckedContinuation<JSONValue, Never>?
    var keychain: CheckedContinuation<String?, Never>?
    var failLogin = false
    var failRestore = false
    var suppliedSession: String?
    var libraryFailures: [Int] = []
    var automaticLibrary = false
    var loginCount = 0
    var libraryCount = 0
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "connect" {
            loginCount += 1
            suppliedSession = body["session_token"].string
            if failRestore { throw RuntimeFailure(message: "核心离线", status: 503) }
            if failLogin { throw RuntimeFailure(message: "账号或密码错误", status: 403) }
            return .object(["core_id": .string("presentation-qa"), "address": .string("http://127.0.0.1:1")])
        }
        if operation == "request", body["path"].text.hasPrefix("/library/tracks?") {
            libraryCount += 1
            if !libraryFailures.isEmpty { throw RuntimeFailure(message: "首次曲库请求失败", status: libraryFailures.removeFirst()) }
            if automaticLibrary { return .object(["items": .array([]), "total": .number(37)]) }
            return await withCheckedContinuation { library = $0 }
        }
        return .object(["id": .string("qa"), "name": .string("QA"), "host_id": .string("qa"), "items": .array([]), "total": .number(0), "track_ids": .array([]), "jobs": .object([:]), "used_bytes": .number(0), "limit_bytes": .number(1024)])
    }
    func shutdown() async {}
}

@Test @MainActor func loginShowsWorkspaceWhileTheFirstLibraryReadIsPendingThenReturnsOnLogout() async throws {
    let runtime = LoginPresentationRuntime(), store = MusicStore(runtime: runtime)
    store.profile = CoreProfile(); store.lanAddress = "http://127.0.0.1:1"
    store.username = "synthetic"; store.password = "synthetic"
    let login = Task { await store.connect() }
    let deadline = ContinuousClock.now + .seconds(3)
    while runtime.library == nil, ContinuousClock.now < deadline { await Task.yield() }
    try #require(runtime.library != nil)
    #expect(store.connected && store.connecting && store.workspaceReady)
    runtime.library?.resume(returning: .object(["items": .array([]), "total": .number(0)])); runtime.library = nil
    await login.value
    #expect(store.workspaceReady)
    await store.handle(RuntimeFailure(message: "暂时离线", status: 503))
    #expect(store.workspaceReady && !store.coreReachable)
    await store.disconnect()
    #expect(!store.workspaceReady)
    await store.shutdown()
}

@Test @MainActor func authenticatedStartupRecoversLibraryWithoutAnotherLogin() async throws {
    let runtime = LoginPresentationRuntime(), store = MusicStore(runtime: runtime)
    runtime.libraryFailures = [503]; runtime.automaticLibrary = true
    store.profile = CoreProfile(); store.lanAddress = "http://127.0.0.1:1"
    store.username = "synthetic"; store.password = "synthetic"
    await store.connect()
    defer { store.stopPolling() }
    try #require(store.workspaceReady && !store.coreReachable && store.connectionIssue != nil)
    #expect(store.password.isEmpty)
    let deadline = ContinuousClock.now + .seconds(9)
    while store.total != 37, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(50)) }
    #expect(store.total == 37 && store.coreReachable && store.connectionIssue == nil)
    #expect(runtime.loginCount == 1 && runtime.libraryCount == 2 && !store.collection.favoriteID.isEmpty)
    await store.shutdown()
}

@Test @MainActor func unauthorizedStartupReturnsToLoginWithoutRetryingLibrary() async {
    let runtime = LoginPresentationRuntime(), store = MusicStore(runtime: runtime)
    runtime.libraryFailures = [401]; runtime.automaticLibrary = true
    store.profile = CoreProfile(); store.lanAddress = "http://127.0.0.1:1"
    store.username = "synthetic"; store.password = "synthetic"
    await store.connect()
    #expect(!store.workspaceReady && !store.connected && store.error?.contains("会话已失效") == true)
    #expect(runtime.loginCount == 1 && runtime.libraryCount == 1)
    await store.shutdown()
}

@Test @MainActor func failedLoginNeverExposesWorkspaceAndKeepsItsInlineError() async {
    let runtime = LoginPresentationRuntime(), store = MusicStore(runtime: runtime)
    runtime.failLogin = true; store.profile = CoreProfile(); store.lanAddress = "http://127.0.0.1:1"
    store.username = "synthetic"; store.password = "synthetic"
    await store.connect()
    #expect(!store.workspaceReady && !store.connecting && !store.connected)
    #expect(store.error == "账号、密码或两步验证码不正确，请检查后重试。")
    await store.shutdown()
}

@Test @MainActor func savedSessionOpensWorkspaceAndCanRecoverAfterCoreIsUnavailable() async throws {
    let runtime = LoginPresentationRuntime()
    runtime.failRestore = true; runtime.automaticLibrary = true
    let store = MusicStore(runtime: runtime, readSavedSession: {
        await withCheckedContinuation { runtime.keychain = $0 }
    })
    store.profile = CoreProfile(); store.lanAddress = "http://127.0.0.1:1"
    store.username = "synthetic"
    let restore = Task { await store.prepare() }
    let deadline = ContinuousClock.now + .seconds(3)
    while runtime.keychain == nil, ContinuousClock.now < deadline { await Task.yield() }
    try #require(runtime.keychain != nil)
    #expect(store.workspaceReady && !store.connected && runtime.loginCount == 0)
    runtime.keychain?.resume(returning: "fixture-session"); runtime.keychain = nil
    await restore.value
    #expect(store.startupUnavailable && !store.workspaceReady)
    #expect(runtime.suppliedSession == "fixture-session")
    runtime.failRestore = false
    await store.retrySavedConnection()
    #expect(store.workspaceReady && store.connected && !store.startupUnavailable)
    await store.disconnect()
    #expect(!store.workspaceReady && !store.startupUnavailable)
    await store.shutdown()
}

@Test func savedCoreSessionIsScopedAndPrivateAcrossLaunches() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = SavedCoreSession(directory: directory)
    try store.save("fixture-session", core: "one", username: "Listener")
    let reopened = SavedCoreSession(directory: directory)
    #expect(reopened.read(core: "one", username: "listener") == "fixture-session")
    #expect(reopened.read(core: "two", username: "listener") == nil)
    #expect(reopened.read(core: "one", username: "another") == nil)
    let permissions = try FileManager.default.attributesOfItem(atPath: directory.appendingPathComponent("core.json").path)
    #expect(permissions[.posixPermissions] as? Int == 0o600)
    reopened.remove()
    #expect(store.read(core: "one", username: "Listener") == nil)
}
