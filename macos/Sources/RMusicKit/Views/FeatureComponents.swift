import SwiftUI

struct FeatureHeader: View {
    let title: String
    var subtitle = ""
    var loading = false
    var refresh: (() -> Void)?
    var body: some View {
        HStack(alignment: .bottom) {
            VStack(alignment: .leading, spacing: 9) {
                Text(title).font(.system(size: 18, weight: .semibold))
                if !subtitle.isEmpty { Text(subtitle).font(.system(size: 12)).foregroundStyle(.secondary) }
            }
            Spacer()
            if loading { ProgressView().controlSize(.small) }
            if let refresh { Button(action: refresh) { Label("更新内容", systemImage: "arrow.clockwise").padding(.horizontal, 8).padding(.vertical, 6).contentShape(Rectangle()) }.buttonStyle(.borderless).disabled(loading) }
        }.padding(.bottom, 16)
    }
}
struct ResourceNotice: View {
    let resource: ResourceStore
    var body: some View {
        if let error = resource.error {
            Label("更新失败，保留已载入内容：\(error)", systemImage: "exclamationmark.circle")
                .font(.callout).foregroundStyle(.orange).textSelection(.enabled).padding(.vertical, 10)
        }
    }
}
struct FeatureCard<Content: View>: View {
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder let content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 14) { content }
            .frame(maxWidth: .infinity, alignment: .leading).padding(20)
            .background(MusicStyle.panel(scheme), in: RoundedRectangle(cornerRadius: AppearanceStore.shared.skin.cornerRadius))
    }
}
struct Fact: View {
    let name: String
    let value: String
    var body: some View {
        if !value.isEmpty { HStack(alignment: .top) { Text(name).foregroundStyle(.secondary).frame(width: 95, alignment: .leading); Text(value).textSelection(.enabled); Spacer(minLength: 0) }.font(.system(size: 12)) }
    }
}
