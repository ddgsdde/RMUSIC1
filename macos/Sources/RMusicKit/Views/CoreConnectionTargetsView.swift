import SwiftUI

struct CoreConnectionTargetsView: View {
    let store: MusicStore
    let local: CoreConnectionInspector
    let selectedID: String?
    let refresh: () -> Void
    @Environment(\.colorScheme) private var scheme
    private var targets: [CoreConnectionTarget] { CoreConnectionTarget.choices(profile: store.profile, local: local.identity, discovered: store.discovery.cores) }
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                Text("选择 Core").font(.title3.weight(.semibold))
                Spacer()
                Button("重新查找 Core", systemImage: "arrow.clockwise", action: refresh).labelStyle(.iconOnly).buttonStyle(MusicControlStyle()).disabled(store.connecting)
            }
            if targets.isEmpty {
                VStack(alignment: .leading, spacing: 10) {
                    Image(systemName: "server.rack").font(.system(size: 28, weight: .light)).foregroundStyle(MusicStyle.accent)
                    Text(store.discovery.searching || local.checking ? "正在查找 Core…" : "暂未发现附近的 Core").font(.callout)
                    Text("也可以直接输入服务器地址。").font(.caption).foregroundStyle(.secondary)
                }.padding(.vertical, 16)
            } else {
                ForEach(targets) { target in targetCard(target) }
            }
            HStack(spacing: 7) {
                if store.discovery.searching || local.checking { ProgressView().controlSize(.mini); Text("正在检查本机与局域网") }
                else { Image(systemName: "network"); Text(store.discovery.error == nil ? "局域网自动发现" : "局域网发现暂不可用") }
            }.font(.caption2).foregroundStyle(.secondary)
        }.padding(26).background(MusicStyle.panel(scheme), in: RoundedRectangle(cornerRadius: 24))
            .overlay(RoundedRectangle(cornerRadius: 24).strokeBorder(.primary.opacity(0.05)))
    }
    private func targetCard(_ target: CoreConnectionTarget) -> some View {
        let selected = CoreConnectionAddress.normalize(store.lanAddress) == target.address && selectedID == target.id
        return Button { store.selectConnectionTarget(target) } label: {
            VStack(alignment: .leading, spacing: 13) {
                HStack {
                    Image(systemName: target.local ? "desktopcomputer" : "server.rack").font(.system(size: 22, weight: .light)).foregroundStyle(MusicStyle.accent)
                    Spacer()
                    Text(target.local ? "这台 Mac" : target.saved ? "上次连接" : "局域网").font(.caption2).foregroundStyle(.secondary)
                    if selected { Image(systemName: "checkmark.circle.fill").foregroundStyle(MusicStyle.accent) }
                }
                Text(target.name).font(.system(size: 15, weight: .semibold)).lineLimit(2)
                Text(target.address).font(.system(size: 10, design: .monospaced)).foregroundStyle(.secondary).lineLimit(2)
            }.frame(maxWidth: .infinity, alignment: .leading).padding(18)
                .background(MusicStyle.accent.opacity(selected ? 0.09 : 0.025), in: RoundedRectangle(cornerRadius: 16))
                .overlay(RoundedRectangle(cornerRadius: 16).stroke(selected ? MusicStyle.accent.opacity(0.55) : .primary.opacity(0.07)))
                .contentShape(RoundedRectangle(cornerRadius: 16))
        }.buttonStyle(.plain).disabled(store.connecting).accessibilityLabel("选择 \(target.name)，\(target.address)")
    }
}
