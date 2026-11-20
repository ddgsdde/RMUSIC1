import SwiftUI

/// Ambient motion is decorative; audio-reactive visuals receive measured data separately.
struct ListeningBackdrop: View {
    let palette: ArtworkPalette
    let theme: ListeningTheme
    let moving: Bool
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        TimelineView(.animation(minimumInterval: 1 / 24, paused: !moving)) { timeline in
            let time = moving ? timeline.date.timeIntervalSinceReferenceDate * 0.055 : 0
            Canvas { context, size in
                let dark = scheme == .dark
                context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(dark ? Color(red: 0.035, green: 0.045, blue: 0.06) : Color(red: 0.94, green: 0.935, blue: 0.92)))
                for index in 0..<3 {
                    let phase = Double(index) * 2.1 + time
                    let center = CGPoint(x: size.width * (0.5 + 0.3 * cos(phase)), y: size.height * (0.5 + 0.35 * sin(phase * 0.83)))
                    let radius = max(size.width, size.height) * 0.85
                    let color = palette.swatches[index].color.opacity(theme == .paper ? 0.12 : dark ? 0.38 : 0.48)
                    context.fill(Path(CGRect(origin: .zero, size: size)), with: .radialGradient(Gradient(colors: [color, .clear]), center: center, startRadius: 0, endRadius: radius))
                }
                // Fine etched curves give the silk depth without giant blur layers or offscreen textures.
                if theme == .silk {
                    for i in 0..<9 {
                        var line = Path(); let y = size.height * (Double(i) / 8)
                        line.move(to: CGPoint(x: 0, y: y))
                        line.addCurve(to: CGPoint(x: size.width, y: y + 180), control1: CGPoint(x: size.width * 0.3, y: y - 100 * cos(time)), control2: CGPoint(x: size.width * 0.7, y: y + 200 * sin(time)))
                        context.stroke(line, with: .color(.primary.opacity(0.018)), lineWidth: 1)
                    }
                }
            }
        }.accessibilityHidden(true).allowsHitTesting(false)
    }
}
