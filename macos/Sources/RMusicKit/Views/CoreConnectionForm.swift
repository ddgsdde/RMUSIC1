import SwiftUI

struct CoreConnectionForm: View {
    @Bindable var store: MusicStore
    let inspector: CoreConnectionInspector
    let retry: () -> Void
    @StoredState private var advanced = false
    @StoredState private var switching = false
    @StoredState private var pending: CoreIdentity?
    @Environment(\.colorScheme) private var scheme
    private var target: CoreIdentity? {
        guard let value = inspector.identity, CoreConnectionAddress.normalize(value.address) == CoreConnectionAddress.normalize(store.lanAddress) else { return nil }
        return value
    }
    private var mismatch: Bool { target?.differs(from: store.connectionBinding) == true }
    private var canSubmit: Bool {
        guard !store.connecting, !inspector.checking, CoreConnectionAddress.normalize(store.lanAddress) != nil, target?.initialized != false else { return false }
        guard target != nil || inspector.error != nil else { return false }
        return mismatch || !store.username.isEmpty && !store.password.isEmpty
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            Text("登录 Core").font(.title3.weight(.semibold))
            field("服务器地址") {
                HStack {
                    TextField("http://192.168.1.10:18854", text: $store.lanAddress).accessibilityIdentifier("coreAddress").onSubmit(retry)
                    Button("检查地址", systemImage: "arrow.clockwise", action: retry).labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).disabled(store.connecting || inspector.checking)
                }
            }
            status
            if URL(string: CoreConnectionAddress.normalize(store.lanAddress) ?? "")?.scheme == "https" { CertificateTrustToggle(address: store.lanAddress, store: store) }
            field("账号") { TextField("用户名", text: $store.username).textContentType(.username).accessibilityIdentifier("username") }.disabled(mismatch)
            field("密码") { SecureField("输入这台 Core 的密码", text: $store.password).textContentType(.password).accessibilityIdentifier("password").onSubmit(connect) }.disabled(mismatch)
            DisclosureGroup(store.bridgeOnly ? "两步验证" : "备用地址与两步验证", isExpanded: $advanced) {
                VStack(spacing: 14) {
                    if !store.bridgeOnly {
                        field("外网备用地址") { TextField("https://music.example.com", text: $store.remoteAddress) }
                    }
                    if !store.bridgeOnly && store.remoteAddress.lowercased().hasPrefix("https://") { CertificateTrustToggle(address: store.remoteAddress, store: store) }
                    field("两步验证码") { TextField("六位验证码（可选）", text: $store.totp) }
                }.padding(.top, 14)
            }.font(.caption).foregroundStyle(.secondary)
            if let error = store.error, !mismatch { Label(error, systemImage: "exclamationmark.circle").font(.caption).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true) }
            Button(action: connect) {
                HStack { if store.connecting { ProgressView().controlSize(.mini) }; Text(store.connecting ? "正在连接音乐库…" : mismatch ? "切换到这台 Core" : "登录"); Spacer(); Image(systemName: "arrow.right") }.frame(maxWidth: .infinity)
            }.buttonStyle(WorkspacePrimaryButton()).disabled(!canSubmit).accessibilityIdentifier("connect").keyboardShortcut(.defaultAction)
        }.padding(28).background(MusicStyle.panel(scheme), in: RoundedRectangle(cornerRadius: 24))
            .overlay(RoundedRectangle(cornerRadius: 24).strokeBorder(.primary.opacity(0.05)))
            .disabled(store.connecting)
            .confirmationDialog("切换 Core", isPresented: $switching, presenting: pending) { identity in
                Button("使用这台 Core") { store.adoptConnectionIdentity(identity); pending = nil }
                Button("取消", role: .cancel) { pending = nil }
            } message: { identity in
                Text("原绑定：\(store.profile.name)（\(store.profile.core_id?.prefix(8) ?? "")）\n当前地址：\(identity.name)（\(identity.core_id.prefix(8))）\n切换后请重新输入这台 Core 的密码。")
            }
    }
    @ViewBuilder private var status: some View {
        if inspector.checking { HStack(spacing: 9) { ProgressView().controlSize(.mini); Text("正在检查 Core…") }.font(.caption).foregroundStyle(.secondary) }
        else if let target {
            VStack(alignment: .leading, spacing: 7) {
                Label(mismatch ? "此地址对应另一台 Core" : "已找到 \(target.name)", systemImage: mismatch ? "exclamationmark.shield" : "checkmark.circle.fill").foregroundStyle(mismatch ? Color.orange : Color.green)
                if mismatch { Text("原绑定：\(store.profile.name) · \(store.profile.core_id?.prefix(8) ?? "")"); Text("当前：\(target.name) · \(target.core_id.prefix(8))") }
                else { Text([target.local ? "本机" : "远程", target.version.isEmpty ? nil : "v\(target.version)", inspector.latencyMS.map { "\($0) ms" }].compactMap { $0 }.joined(separator: " · ")).foregroundStyle(.secondary) }
                if target.initialized == false { HStack { Text("这台 Core 尚未初始化"); if let url = URL(string: target.address) { Link("打开初始化页面", destination: url) } } }
            }.font(.caption).padding(14).frame(maxWidth: .infinity, alignment: .leading).background((mismatch ? Color.orange : MusicStyle.accent).opacity(0.06), in: RoundedRectangle(cornerRadius: 12))
        } else if let error = inspector.error { HStack { Label(error, systemImage: "wifi.exclamationmark"); Spacer(); Button("重试", action: retry).buttonStyle(.plain) }.font(.caption).foregroundStyle(.secondary) }
    }
    private func field<Content: View>(_ name: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 9) { Text(name).font(.caption).foregroundStyle(.secondary); content().font(.system(size: 13)).textFieldStyle(.plain).padding(13).background(.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 11)) }
    }
    private func connect() {
        guard canSubmit else { return }
        if mismatch, let target { pending = target; switching = true }
        else { Task { await store.connect() } }
    }
}
