import SwiftUI

struct FavoriteButton: View {
    let track: Track
    let store: MusicStore
    var size: CGFloat = 18
    @StoredState private var wanted: Bool?
    @StoredState private var burst = 0
    @StoredState private var sparks = false
    @StoredState private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var liked: Bool { wanted ?? store.collection.favorites.contains(track.id) }
    var body: some View {
        Button {
            let selected = !liked
            wanted = selected
            if selected && !reduceMotion { burst += 1 }
            Task {
                await store.perform { try await store.collection.like(track, store: store) }
                wanted = nil
            }
        } label: {
            Image(systemName: liked ? "heart.fill" : "heart")
                .font(.system(size: size, weight: .medium))
                .foregroundStyle(liked ? MusicStyle.accent : Color.secondary)
                .symbolEffect(.bounce, options: .speed(1.6), value: burst)
                .frame(width: 36, height: 36).contentShape(Rectangle())
                .overlay {
                    if sparks {
                        ZStack {
                            ForEach(0..<6) { index in
                                let angle = Double(index) * .pi / 3 - .pi / 12
                                Circle().fill(MusicStyle.accent).frame(width: 3, height: 3)
                                    .offset(x: cos(angle) * (expanded ? 26 : 8), y: sin(angle) * (expanded ? 26 : 8))
                            }
                        }.opacity(expanded ? 0 : 0.8).allowsHitTesting(false)
                    }
                }
        }.buttonStyle(MusicButtonStyle()).disabled(wanted != nil)
            .accessibilityLabel(liked ? "取消喜欢 \(track.title)" : "喜欢 \(track.title)")
            .accessibilityValue(liked ? "已喜欢" : "未喜欢")
            .help(liked ? "取消喜欢" : "喜欢这首歌")
            .task(id: burst) {
                guard burst > 0, !reduceMotion else { return }
                sparks = true; expanded = false
                await Task.yield()
                withAnimation(.easeOut(duration: 0.55)) { expanded = true }
                try? await Task.sleep(for: .milliseconds(600))
                sparks = false
            }
    }
}
