import CoreGraphics
import SwiftUI

struct ArtworkPalette: Sendable, Equatable {
    struct Swatch: Sendable, Equatable {
        var r: Double; var g: Double; var b: Double
        var color: Color { Color(red: r, green: g, blue: b) }
    }
    var swatches: [Swatch]
    static let fallback = Self(swatches: [.init(r: 0.30, g: 0.39, b: 0.57), .init(r: 0.58, g: 0.48, b: 0.65), .init(r: 0.33, g: 0.57, b: 0.60)])
    /// A bounded, colour-managed 24×24 sample; never samples a full-size image on the UI thread.
    static func extract(_ image: CGImage) -> Self {
        let side = 24
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        let success = pixels.withUnsafeMutableBytes { bytes -> Bool in
            guard let context = CGContext(data: bytes.baseAddress, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side)); return true
        }
        guard success else { return .fallback }
        var bins = [Int: (r: Double, g: Double, b: Double, count: Int)]()
        for i in stride(from: 0, to: pixels.count, by: 4) {
            let alpha = Double(pixels[i + 3]) / 255
            guard alpha > 0.5 else { continue }
            let r = Double(pixels[i]) / 255 / alpha, g = Double(pixels[i + 1]) / 255 / alpha, b = Double(pixels[i + 2]) / 255 / alpha
            let luma = r * 0.2126 + g * 0.7152 + b * 0.0722
            guard luma > 0.04, luma < 0.94 else { continue }
            let key = Int(r * 5) * 36 + Int(g * 5) * 6 + Int(b * 5)
            let old = bins[key] ?? (0, 0, 0, 0)
            bins[key] = (old.r + r, old.g + g, old.b + b, old.count + 1)
        }
        let sorted = bins.sorted { $0.value.count == $1.value.count ? $0.key < $1.key : $0.value.count > $1.value.count }
        var result: [Swatch] = []
        for item in sorted {
            let n = Double(item.value.count)
            let c = Swatch(r: item.value.r / n, g: item.value.g / n, b: item.value.b / n)
            if result.allSatisfy({ abs($0.r-c.r)+abs($0.g-c.g)+abs($0.b-c.b) > 0.3 }) { result.append(c) }
            if result.count == 3 { break }
        }
        guard let first = result.first else { return .fallback }
        while result.count < 3 { result.append(first) }
        return Self(swatches: result)
    }
}
