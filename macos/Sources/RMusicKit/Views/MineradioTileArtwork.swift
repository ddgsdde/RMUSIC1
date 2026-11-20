import SwiftUI

/// Static layered artwork from the upstream home tile vocabulary; no extra animation clock.
struct MineradioTileArtwork: View {
    let symbol: String
    var featured = false
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 18).fill(LinearGradient(colors: [Color(red: 0.66, green: 0.80, blue: 0.84), Color(red: 0.35, green: 0.43, blue: 0.61), Color(red: 0.22, green: 0.24, blue: 0.40)], startPoint: .topLeading, endPoint: .bottomTrailing))
                .overlay(RoundedRectangle(cornerRadius: 18).stroke(.white.opacity(0.13), lineWidth: 1))
            Circle().fill(AngularGradient(colors: [.white.opacity(0.03), .white.opacity(0.34), Color(red: 0.51, green: 0.60, blue: 0.87).opacity(0.5), .white.opacity(0.08), .white.opacity(0.30), .white.opacity(0.03)], center: .center)).padding(11)
                .overlay(Circle().stroke(.white.opacity(0.13), lineWidth: 0.5).padding(19))
            Circle().fill(Color(red: 0.60, green: 0.74, blue: 0.77).opacity(0.55)).padding(26)
            Image(systemName: symbol).font(.system(size: 12, weight: .light)).foregroundStyle(.white.opacity(0.4))
        }.frame(width: featured ? 74 : 56, height: featured ? 74 : 56)
            .rotation3DEffect(.degrees(-16), axis: (x: 1, y: 1, z: 0))
            .shadow(color: .black.opacity(0.22), radius: 15, x: 5, y: 10)
            .accessibilityHidden(true)
    }
}
