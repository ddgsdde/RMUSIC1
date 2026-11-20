import AppKit
import CoreGraphics
import Foundation
import Testing
@testable import RMusicKit

@Test @MainActor func workspaceSkinIsIndependentFromPaletteAndOldPreferencesStillLoad() {
    let suite = "rmusic-appearance-\(UUID())"
    let defaults = UserDefaults(suiteName: suite)!
    defer { defaults.removePersistentDomain(forName: suite) }
    let appearance = AppearanceStore(defaults: defaults)
    #expect(appearance.skin == .studio)
    appearance.skin = .mineradio; appearance.theme = .jade
    let restored = AppearanceStore(defaults: defaults)
    #expect(restored.skin == .mineradio && restored.theme == .jade)
    #expect(restored.scheme == .dark)
}
@Test func paletteComesFromActualPixelsAndFeatureValidationRejectsMalformedNumbers() throws {
    let context = try #require(CGContext(data:nil,width:12,height:12,bitsPerComponent:8,bytesPerRow:48,space:CGColorSpace(name: CGColorSpace.sRGB)!,bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue))
    context.setFillColor(CGColor(colorSpace:CGColorSpace(name:CGColorSpace.sRGB)!,components:[0.2,0.4,0.7,1])!);context.fill(CGRect(x:0,y:0,width:12,height:12))
    let result = ArtworkPalette.extract(try #require(context.makeImage()))
    #expect(abs(result.swatches[0].r-0.2)<0.02 && abs(result.swatches[0].b-0.7)<0.02)
    #expect(!VisualFeature(playback_id:"old",position:.nan,bands:[Int](repeating:0,count:8),energy:1,peak:1).valid)
    #expect(!VisualFeature(playback_id:"p",position:0,bands:[256],energy:1,peak:1).valid)
}

@Test @MainActor func mineradioPaletteChangesTheRenderedAccentInsteadOfOnlyPersistingTheChoice() throws {
    let appearance = AppearanceStore.shared
    let previousSkin = appearance.skin, previousTheme = appearance.theme
    defer { appearance.skin = previousSkin; appearance.theme = previousTheme }
    appearance.skin = .mineradio
    appearance.theme = .amber
    let amber = try #require(NSColor(MusicStyle.accent).usingColorSpace(.deviceRGB))
    appearance.theme = .ocean
    let ocean = try #require(NSColor(MusicStyle.accent).usingColorSpace(.deviceRGB))
    #expect(amber.redComponent > ocean.redComponent + 0.2)
    #expect(ocean.blueComponent > amber.blueComponent + 0.2)
}
