import SwiftUI

struct ListeningRecord: View {
    let store: MusicStore
    let size: CGFloat
    let moving: Bool
    let paper: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @StoredState private var started = Date()
    @StoredState private var accumulated = 0.0
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: paper ? 8 : 30)
                .fill(.primary.opacity(paper ? 0.035 : 0.025))
                .overlay { RoundedRectangle(cornerRadius: paper ? 8 : 30).strokeBorder(.primary.opacity(0.09), lineWidth: 1) }
                .shadow(color: .black.opacity(0.12), radius: 24, y: 18)
            TimelineView(.animation(minimumInterval: 1 / 30, paused: !moving)) { timeline in
                let phase = accumulated + (moving ? timeline.date.timeIntervalSince(started) : 0)
                ZStack {
                    Circle().fill(LinearGradient(colors: [.primary.opacity(0.04), .primary.opacity(0.15), .primary.opacity(0.03)], startPoint: .topLeading, endPoint: .bottomTrailing))
                    ForEach(0..<22, id: \.self) { index in Circle().stroke(.primary.opacity(index % 3 == 0 ? 0.09 : 0.035), lineWidth: 0.7).padding(CGFloat(index) * size * 0.009) }
                    ArtworkView(track: store.zone?.track, store: store, radius: size, resolveMissing: true).frame(width: size * 0.47, height: size * 0.47)
                    Circle().fill(.primary.opacity(0.7)).frame(width: 9, height: 9)
                }.padding(size * 0.09).rotationEffect(.degrees(phase * 5))
            }
            // Brushed tonearm: static geometry, no simulated material blur.
            tonearm.frame(width: size * 0.16, height: size * 0.8).offset(x: size * 0.35, y: -size * 0.08).rotationEffect(.degrees(moving ? 0 : -12), anchor: .top)
        }.frame(width: size, height: size)
        .onChange(of: moving) { _, new in
            if new { started = Date() } else { accumulated += Date().timeIntervalSince(started) }
        }
        .onChange(of: store.zone?.track?.id) { accumulated = 0; started = Date() }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.65), value: moving)
        .accessibilityHidden(true)
    }
    private var tonearm: some View {
        GeometryReader { geo in
            ZStack(alignment: .top) {
                Path { p in p.move(to: CGPoint(x: geo.size.width / 2, y: 18)); p.addLine(to: CGPoint(x: geo.size.width / 2, y: geo.size.height * 0.7)); p.addQuadCurve(to: CGPoint(x: 8, y: geo.size.height - 16), control: CGPoint(x: geo.size.width / 2, y: geo.size.height * 0.9)) }
                    .stroke(LinearGradient(colors: [.gray, .white.opacity(0.9), .gray.opacity(0.7)], startPoint: .leading, endPoint: .trailing), style: StrokeStyle(lineWidth: 6, lineCap: .round)).shadow(color: .black.opacity(0.2), radius: 3, y: 2)
                Circle().fill(.gray.gradient).frame(width: 30, height: 30).overlay { Circle().stroke(.white.opacity(0.5), lineWidth: 2).padding(7) }
                Capsule().fill(.gray.gradient).frame(width: 17, height: 35).rotationEffect(.degrees(30)).offset(x: -geo.size.width / 2 + 10, y: geo.size.height - 38)
            }
        }
    }
}
