import SwiftUI

/// Sparse overlay above the original camera and GPU lyric scene; never duplicates the scene with flat lyrics.
struct MineradioStageView: View {
    let store: MusicStore
    @StoredState private var appearance = false
    @StoredState private var lyrics = false
    var body: some View {
        VStack {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 8) {
                    Text(store.zone.map { store.outputLabels.label($0) } ?? "选择声音设备").font(.caption).foregroundStyle(.secondary).lineLimit(2)
                    Text(store.zone?.track?.title ?? "尚未播放").font(.system(size: 18, weight: .medium)).lineLimit(2)
                    Group { if let track = store.zone?.track { CatalogTextLink(track: track, store: store) } }.font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }.frame(maxWidth: 340, alignment: .leading)
                Spacer()
                Button("歌词", systemImage: "textformat") { lyrics.toggle() }.buttonStyle(MineradioCircleButton()).labelStyle(.iconOnly).help("歌词与翻译")
                Button("视觉调节", systemImage: "slider.horizontal.3") { appearance = true }.buttonStyle(MineradioCircleButton()).labelStyle(.iconOnly).help("视觉调节")
            }
            Spacer()
        }.padding(.top, 2).frame(maxWidth: .infinity, maxHeight: .infinity)
            .overlay {
                if lyrics {
                    GeometryReader { geometry in
                        VStack(alignment: .leading, spacing: 18) {
                            HStack {
                                Text("歌词与翻译").font(.headline)
                                Spacer()
                                Button("收起歌词", systemImage: "xmark") { lyrics = false }.labelStyle(.iconOnly).buttonStyle(MusicControlStyle())
                            }
                            ListeningLyrics(store: store)
                        }.padding(20)
                            .frame(width: min(420, max(0, geometry.size.width - 8)), height: min(540, max(0, geometry.size.height - 64)))
                            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 20))
                            .overlay(RoundedRectangle(cornerRadius: 20).stroke(.white.opacity(0.12)))
                            .shadow(color: .black.opacity(0.3), radius: 24, y: 12)
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing).padding(.top, 56)
                    }.environment(\.colorScheme, .dark)
                }
            }
            .sheet(isPresented: $appearance) { MineradioConsoleView() }
    }
}
