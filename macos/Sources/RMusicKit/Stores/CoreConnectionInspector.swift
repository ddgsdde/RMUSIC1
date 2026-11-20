import Foundation
import Observation

/// The view owns and cancels each probe. No account data is included in status inspection.
@MainActor @Observable final class CoreConnectionInspector {
    var identity: CoreIdentity?
    var latencyMS: Int?
    var checking = false
    var error: String?
    @ObservationIgnored private var revision = UUID()
    func clear() { revision = UUID(); identity = nil; checking = false; error = nil }
    func inspect(_ profile: CoreProfile, store: MusicStore, debounce: Bool = true) async {
        let request = UUID(); revision = request; identity = nil; latencyMS = nil; error = nil
        guard let address = CoreConnectionAddress.normalize(profile.address) else {
            checking = false
            if !profile.address.isEmpty { error = "请填写有效的 HTTP 或 HTTPS 地址。" }
            return
        }
        checking = true
        defer { if revision == request { checking = false } }
        do {
            if debounce { try await Task.sleep(for: .milliseconds(350)) }
            var candidate = profile; candidate.address = address; candidate.alternatives = []
            try await LocalNetworkReadiness.check(address)
            let start = ProcessInfo.processInfo.systemUptime
            let value = try await store.runtimeCall("inspect_core", try .encoded(candidate)).decode(CoreIdentity.self)
            try Task.checkCancellation()
            guard revision == request else { return }
            latencyMS = Int(((ProcessInfo.processInfo.systemUptime - start) * 1000).rounded())
            identity = value
        } catch is CancellationError {} catch {
            guard revision == request, !Task.isCancelled else { return }
            self.error = "此地址暂时无法访问。\(error.localizedDescription)"
        }
    }
}

extension MusicStore {
    /// A confirmed switch remains a draft until authentication succeeds.
    var connectionBinding: CoreProfile {
        guard let identity = connectionIdentity, CoreConnectionAddress.normalize(lanAddress) == identity.address else { return profile }
        return CoreProfile(name: identity.name, address: identity.address, core_id: identity.core_id)
    }
    func selectConnectionTarget(_ target: CoreConnectionTarget) {
        guard !connected, !connecting else { return }
        if CoreConnectionAddress.normalize(lanAddress) != target.address { password = ""; totp = ""; remoteAddress = "" }
        lanAddress = target.address
        discovery.selectedID = discovery.cores.first { ($0.core_id ?? $0.id) == target.id }?.id
        error = nil
        if target.saved { connectionIdentity = nil; remoteAddress = bridgeOnly ? "" : (profile.alternatives.first { $0.from_core != true }?.address ?? ""); connectionTrust = ConnectionTrustPreferences(profile: profile) }
    }
    func adoptConnectionIdentity(_ identity: CoreIdentity) {
        guard !connected, !connecting, CoreConnectionAddress.normalize(lanAddress) == CoreConnectionAddress.normalize(identity.address) else { return }
        let skip = connectionTrust.allowed(for: identity.address)
        connectionIdentity = identity
        connectionTrust = ConnectionTrustPreferences(profile: CoreProfile(address: identity.address, skip_certificate_verification: skip, core_id: identity.core_id))
        remoteAddress = ""; password = ""; totp = ""; error = nil; discovery.selectedID = nil
    }
}
