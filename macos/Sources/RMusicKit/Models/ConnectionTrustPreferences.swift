import Foundation

/// Exceptions belong to an exact HTTPS entry point, never a host-wide or process-wide switch.
struct ConnectionTrustPreferences {
    private var exceptions: Set<String> = []
    init(profile: CoreProfile = CoreProfile()) {
        if profile.skip_certificate_verification == true { set(true, for: profile.address) }
        for address in profile.alternatives where address.skip_certificate_verification == true { set(true, for: address.address) }
    }
    static func key(_ address: String) -> String? {
        guard var parts = URLComponents(string: address.trimmingCharacters(in: .whitespacesAndNewlines)),
              parts.scheme?.lowercased() == "https", let host = parts.host, !host.isEmpty,
              parts.user == nil, parts.password == nil, parts.query == nil, parts.fragment == nil else { return nil }
        parts.scheme = "https"; parts.host = host.lowercased()
        if parts.port == 443 { parts.port = nil }
        while parts.percentEncodedPath.hasSuffix("/") { parts.percentEncodedPath.removeLast() }
        return parts.url?.absoluteString
    }
    func allowed(for address: String) -> Bool { Self.key(address).map { exceptions.contains($0) } ?? false }
    mutating func set(_ enabled: Bool, for address: String) {
        guard let key = Self.key(address) else { return }
        if enabled { exceptions.insert(key) } else { exceptions.remove(key) }
    }
    mutating func changed(from old: String, to new: String) {
        if Self.key(old) != Self.key(new) { set(false, for: old) }
    }
    func applying(to profile: CoreProfile, primary: String, alternatives: [String]) -> CoreProfile {
        var result = profile
        result.address = primary
        result.skip_certificate_verification = allowed(for: primary)
        result.alternatives = alternatives.map { CoreAddress(address: $0, skip_certificate_verification: allowed(for: $0)) }
        return result
    }
}
