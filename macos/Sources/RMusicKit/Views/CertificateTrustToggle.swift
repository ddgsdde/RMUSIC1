import SwiftUI

struct CertificateTrustToggle: View {
    let address: String
    @Bindable var store: MusicStore
    private var allowed: Bool { store.connectionTrust.allowed(for: address) }
    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            Toggle("跳过 HTTPS 证书校验（不安全）", isOn: Binding(
                get: { allowed }, set: { store.connectionTrust.set($0, for: address) }))
                .toggleStyle(.switch).controlSize(.small)
                .disabled(store.connecting || ConnectionTrustPreferences.key(address) == nil)
            if allowed { Text("仅此地址跳过证书链、有效期和域名校验；连接仍使用 TLS 加密。").font(.caption2).foregroundStyle(.orange) }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}
