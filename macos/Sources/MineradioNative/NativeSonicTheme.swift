import Foundation

/// syncTheme's persistent material colors; cover changes do not replace the entire palette in one frame.
struct NativeSonicTheme {
    private static let keys = ["uBaseColor1","uBaseColor2","uFogColor","uCoolCore","uCoolEdge","uWarmCore","uWarmEdge","uRippleColor","uGlowIntensity"]
    private var colors: [String:[Float]] = [
        "uBaseColor1":[0.01,0.02,0.04],"uBaseColor2":[0.03,0.05,0.09],"uFogColor":[0.01,0.02,0.04],
        "uCoolCore":[0,0.3,1],"uCoolEdge":[0.6,0.2,1],"uWarmCore":[1,0.2,0.1],"uWarmEdge":[1,0.6,0],"uRippleColor":[0.2,0.9,1],"uGlowIntensity":[1]
    ]
    mutating func apply(to values: inout [String:[Float]], dt: Float) {
        let blend = max(0,min(1,3*dt))
        for key in Self.keys {
            guard let target = values[key], var color = colors[key], color.count == target.count else { continue }
            for i in color.indices { color[i] += (target[i]-color[i])*blend }
            colors[key] = color; values[key] = color
        }
    }
}
