import Foundation

struct CoreIdentity: Decodable, Equatable {
    let address: String
    let core_id: String
    let name: String
    let version: String
    let initialized: Bool?
    var local: Bool { ["localhost", "127.0.0.1", "::1", "[::1]"].contains(URL(string: address)?.host ?? "") }
    func differs(from profile: CoreProfile) -> Bool { profile.core_id.map { $0 != core_id } ?? false }
}

enum CoreConnectionAddress {
    static func normalize(_ input: String) -> String? {
        var value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return nil }
        if !value.contains("://") { value = "http://" + value }
        guard var url = URLComponents(string: value), ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
              let host = url.host, !host.isEmpty, !host.contains(where: { $0.isWhitespace || $0.isNewline }),
              url.port.map({ (1...65535).contains($0) }) ?? true,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil else { return nil }
        url.scheme = url.scheme?.lowercased(); url.host = host.lowercased()
        if url.scheme == "http" && url.port == 80 || url.scheme == "https" && url.port == 443 { url.port = nil }
        while url.percentEncodedPath.hasSuffix("/") { url.percentEncodedPath.removeLast() }
        return url.url?.absoluteString
    }
}

struct CoreConnectionTarget: Identifiable {
    let id: String
    let name: String
    let address: String
    let local: Bool
    let saved: Bool
    static func choices(profile: CoreProfile, local: CoreIdentity?, discovered: [DiscoveredCore]) -> [Self] {
        var items: [Self] = [], identities = Set<String>()
        func add(_ id: String, _ name: String, _ address: String, local: Bool, saved: Bool) {
            guard let address = CoreConnectionAddress.normalize(address), !identities.contains(id) else { return }
            identities.insert(id)
            items.append(Self(id: id, name: name, address: address, local: local, saved: saved))
        }
        if let id = profile.core_id {
            let verifiedLocal = local.flatMap { $0.core_id == id ? $0 : nil }
            add(id, profile.name, verifiedLocal?.address ?? profile.address, local: verifiedLocal != nil, saved: true)
        }
        if let local { add(local.core_id, local.name, local.address, local: true, saved: false) }
        for core in discovered { if let address = core.addresses.first { add(core.core_id ?? core.id, core.name, address, local: false, saved: false) } }
        return items
    }
}
