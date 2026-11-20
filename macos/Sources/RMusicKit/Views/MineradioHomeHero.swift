import SwiftUI

struct MineradioHomeHero: View {
    let track: Track?
    let covers: [Track]
    let store: MusicStore
    let action: () -> Void
    let ready: Bool
    let current: Bool
    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                MusicStyle.accent.opacity(0.10)
                if let track { ArtworkView(track: track, store: store, radius: 0).scaledToFill().frame(width: geometry.size.width, height: geometry.size.height).opacity(0.22).clipped() }
                LinearGradient(colors: [.black.opacity(0.60), .black.opacity(0.50), .black.opacity(0.10)], startPoint: .leading, endPoint: .trailing)
                HStack(spacing: 20) {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 6) { Circle().fill(MusicStyle.accent).frame(width: 5, height: 5); Text(current ? "CONTINUE LISTENING" : "FROM YOUR LIBRARY").font(.system(size: 8, weight: .semibold)).tracking(2) }.foregroundStyle(.white.opacity(0.65))
                        Spacer(minLength: 4)
                        Text(track?.title ?? "音乐库").font(.system(size: 30, weight: .medium)).lineLimit(2).minimumScaleFactor(0.75)
                        Group { if let track { CatalogTextLink(track: track, store: store) } }.font(.system(size: 12)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
                        Spacer(minLength: 4)
                        Button(action: action) { Label(ready ? current ? "继续播放" : "开始聆听" : "选择播放设备", systemImage: ready ? "play.fill" : "hifispeaker").font(.system(size: 11, weight: .semibold)).padding(.horizontal, 16).padding(.vertical, 11).background(MusicStyle.accent, in: Capsule()).foregroundStyle(MusicStyle.onAccent) }.buttonStyle(.plain)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                    ZStack {
                        if let other = covers.dropFirst().first { ArtworkView(track: other, store: store, radius: 10).frame(width: 142, height: 142).rotationEffect(.degrees(11)).offset(x: 28, y: -8).opacity(0.5) }
                        ArtworkView(track: track, store: store, radius: 10).frame(width: 166, height: 166).rotationEffect(.degrees(-7)).shadow(color: .black.opacity(0.5), radius: 18, y: 10).overlay(RoundedRectangle(cornerRadius: 10).stroke(.white.opacity(0.12)).rotationEffect(.degrees(-7)))
                    }.frame(width: min(206, geometry.size.width * 0.34))
                }.padding(30)
            }.clipShape(RoundedRectangle(cornerRadius: 24)).overlay(RoundedRectangle(cornerRadius: 24).stroke(LinearGradient(colors: [.white.opacity(0.20), .white.opacity(0.03)], startPoint: .topLeading, endPoint: .bottomTrailing), lineWidth: 1))
        }
    }
}

struct MineradioHomeShortcut: View {
    let title: String
    let caption: String
    let symbol: String
    let artwork: Track?
    let store: MusicStore
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            HStack(spacing: 16) {
                VStack(alignment: .leading, spacing: 10) { Text(caption).font(.system(size: 8, weight: .medium)).tracking(1.2).foregroundStyle(.secondary); Text(title).font(.system(size: 17, weight: .medium)); Image(systemName: "arrow.up.right").font(.system(size: 10)).foregroundStyle(MusicStyle.accent) }
                Spacer(minLength: 0)
                ZStack(alignment: .bottomTrailing) { ArtworkView(track: artwork, store: store, radius: 9).frame(width: 70, height: 70).rotationEffect(.degrees(8)).opacity(0.7); Image(systemName: symbol).font(.system(size: 16, weight: .light)).foregroundStyle(.white).frame(width: 34, height: 34).background(.ultraThinMaterial, in: Circle()).overlay(Circle().stroke(.white.opacity(0.12))).offset(x: 6, y: 6) }
            }.padding(22).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading).background(MineradioMaterial(cornerRadius: 20)).clipShape(RoundedRectangle(cornerRadius: 20)).overlay(RoundedRectangle(cornerRadius: 20).stroke(.white.opacity(0.08)))
        }.buttonStyle(.plain)
    }
}
