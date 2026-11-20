import simd
import Testing
@testable import MineradioNative

@Test func asymmetricLyricContextKeepsTheAuthoredAnchorAndLongLinesStayVisible() {
    let anchor = SIMD4<Float>(0,0,0,1)
    // At the start of a song the visible context has more lines below the current line.
    let initial = nativeLyricViewportFit(minimum:SIMD2(-0.55,-0.76),maximum:SIMD2(0.55,0.24))
    #expect(initial*anchor == anchor)
    // Long bilingual lines must fit without cropping either end of their glyph bounds.
    let low = SIMD2<Float>(-1.8,-1.4), high = SIMD2<Float>(1.8,0.6)
    let fitted = nativeLyricViewportFit(minimum:low,maximum:high)
    for x in [low.x,high.x] { for y in [low.y,high.y] {
        let point = fitted*SIMD4<Float>(x,y,0,1)
        #expect(abs(point.x) <= 0.92001 && abs(point.y) <= 0.88001)
    } }
}
