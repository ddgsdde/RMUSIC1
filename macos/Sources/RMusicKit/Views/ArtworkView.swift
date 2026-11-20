import SwiftUI
import AppKit
import ImageIO

struct ArtworkView: View {
    let track: Track?
    let store: MusicStore
    var radius: CGFloat = 12
    var resolveMissing = false
    @StoredState private var image: NSImage?
    @StoredState private var failed = false
    @StoredState private var retryID = UUID()
    @StoredState private var loadedSource = ""
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                RoundedRectangle(cornerRadius: radius).fill(.quaternary)
                if let image {
                    Image(nsImage: image).resizable().scaledToFill()
                        .frame(width: geometry.size.width, height: geometry.size.height)
                } else {
                    LinearGradient(colors: [MusicStyle.accent.opacity(0.16), Color.orange.opacity(0.04)], startPoint: .topLeading, endPoint: .bottomTrailing)
                    Image(systemName: "opticaldisc").resizable().scaledToFit().padding(min(20, min(geometry.size.width, geometry.size.height) * 0.22)).foregroundStyle(.secondary.opacity(0.5))
                }
            }.frame(width: geometry.size.width, height: geometry.size.height)
        }
        .clipShape(RoundedRectangle(cornerRadius: radius))
        .overlay(alignment: .bottomTrailing) {
            if failed, let track {
                Button { store.artwork.retry(track, store: store); retryID = UUID() } label: { Image(systemName: "arrow.clockwise").font(.caption).padding(7).background(.regularMaterial, in: Circle()) }.buttonStyle(.plain).padding(5).help("重新加载这张图片").accessibilityLabel("重新加载图片")
            }
        }
        .task(id: "\(store.sessionID):\(store.artwork.revision):\(store.artwork.sourceKey(track)):\(retryID)") {
            let source = "\(store.sessionID):\(store.artwork.sourceKey(track))"
            if loadedSource != source { image = nil; loadedSource = source }
            failed = false
            guard let track, track.cover_url?.isEmpty == false || resolveMissing else { return }
            let loaded = await store.artwork.image(track, store: store, resolveMissing: resolveMissing)
            guard !Task.isCancelled else { return }
            if let loaded { image = loaded; return }
            failed = image == nil
            // One visible retry after the shared negative-cache interval. A transient
            // first miss must not remain blank forever until the entire page is refreshed.
            do { try await Task.sleep(for: .seconds(16)) } catch { return }
            let recovered = await store.artwork.image(track, store: store, resolveMissing: resolveMissing)
            guard !Task.isCancelled else { return }
            if let recovered { image = recovered; failed = false }
        }
        .help(image == nil ? ((track?.cover_url?.isEmpty ?? true) ? "尚未取得封面资料，可刷新重试" : "封面暂时无法加载") : "")
        .accessibilityHidden(!failed)
    }
}
