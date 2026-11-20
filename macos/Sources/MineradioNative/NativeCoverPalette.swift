import simd

// Mineradio d43de565, GPL-3.0: fixed-step sampling, area buckets and lyric tones.
// Values stay in the same sRGB range as the pinned Three.js shader inputs.
struct NativeCoverPalette: Sendable {
    var primary = SIMD3<Float>(0.616,0.722,0.812)
    var base = SIMD3<Float>(0.02,0.027,0.047)
    var warm = SIMD3<Float>(0.616,0.722,0.812)
    var cool = SIMD3<Float>(0.616,0.722,0.812)
    var light = SIMD3<Float>(0.933,0.969,1)
    var accent = SIMD3<Float>(0.616,0.722,0.812)
    var lyric = SIMD3<Float>(216,241,255)/255
    var secondary = SIMD3<Float>(157,184,207)/255
    var highlight = SIMD3<Float>(238,247,255)/255
    var monochrome = true

    func blended(to target: Self, amount: Float) -> Self {
        let t = max(0,min(1,amount)); var result = self
        result.primary += (target.primary-primary)*t; result.base += (target.base-base)*t
        result.warm += (target.warm-warm)*t; result.cool += (target.cool-cool)*t
        result.light += (target.light-light)*t; result.accent += (target.accent-accent)*t
        result.lyric += (target.lyric-lyric)*t; result.secondary += (target.secondary-secondary)*t
        result.highlight += (target.highlight-highlight)*t; result.monochrome = t < 0.5 ? monochrome : target.monochrome
        return result
    }

    private struct Sample {
        let rgb: SIMD3<Double>
        let count: Int
        let order: Int
        var lum: Double { dot(rgb,SIMD3(0.299,0.587,0.114))/255 }
        var chroma: Double { (max(rgb.x,max(rgb.y,rgb.z))-min(rgb.x,min(rgb.y,rgb.z)))/255 }
        var hsl: SIMD3<Double> { NativeCoverPalette.hsl(rgb/255) }
        var color: SIMD3<Float> { SIMD3<Float>(rgb)/255 }
    }
    static func extract(_ pixels: [UInt8], width: Int, height: Int) -> Self {
        guard width > 0, height > 0, width <= 256, height <= 256, pixels.count == width*height*4 else { return Self() }
        var sums = [SIMD3<Double>](), counts = [Int](), indices: [Int:Int] = [:]
        var sum = SIMD3<Double>.zero, total = 0, chromaSum = 0.0, maxChroma = 0.0, colorful = 0, usable = 0
        var best: Sample?, bestScore = -1.0
        for y in stride(from:0,to:height,by:8) { for x in stride(from:0,to:width,by:8) {
            let at = (y*width+x)*4
            guard pixels[at+3] >= 128 else { continue }
            let rgb = SIMD3(Double(pixels[at]),Double(pixels[at+1]),Double(pixels[at+2]))
            let sample = Sample(rgb:rgb,count:1,order:total), hsl = sample.hsl, chroma = sample.chroma, lum = sample.lum
            sum += rgb; total += 1; chromaSum += chroma; maxChroma = max(maxChroma,chroma)
            if chroma > 0.055 && hsl.y > 0.075 { colorful += 1 }
            if chroma > 0.080 && hsl.y > 0.10 && lum > 0.08 && lum < 0.92 { usable += 1 }
            let score = chroma*1.6+(0.5-abs(lum-0.5))*0.45
            if lum > 0.08 && lum < 0.92 && score > bestScore { best = sample; bestScore = score }
            let key = Int((rgb.x/24).rounded(.toNearestOrAwayFromZero))*144+Int((rgb.y/24).rounded(.toNearestOrAwayFromZero))*12+Int((rgb.z/24).rounded(.toNearestOrAwayFromZero))
            if let i = indices[key] { sums[i] += rgb; counts[i] += 1 }
            else { indices[key] = sums.count; sums.append(rgb); counts.append(1) }
        } }
        guard total > 0 else { return Self() }
        let average = Sample(rgb:sum/Double(total),count:total,order:0)
        let list = sums.indices.map { i in Sample(rgb:(sums[i]/Double(counts[i])).rounded(.toNearestOrAwayFromZero),count:counts[i],order:i) }.sorted {
            if $0.count != $1.count { return $0.count > $1.count }
            if $0.chroma != $1.chroma { return $0.chroma > $1.chroma }
            return $0.order < $1.order
        }
        func pick(_ fallback: Sample, avoiding: Sample? = nil, distance: Double = 0.06, _ test: (Sample)->Bool) -> Sample {
            list.first { sample in test(sample) && (avoiding.map { length((sample.rgb-$0.rgb)/255) >= distance } ?? true) } ?? fallback
        }
        let primary = pick(list[0]) { $0.lum > 0.035 && $0.lum < 0.965 }
        let base = pick(primary) { $0.lum < min(0.46,(primary.lum == 0 ? 0.5 : primary.lum)+0.10) }
        let warm = pick(primary) { $0.chroma > 0.055 && $0.hsl.y > 0.08 && ($0.hsl.x < 0.18 || $0.hsl.x > 0.90) }
        let cool = pick(primary,avoiding:warm,distance:0.055) { $0.chroma > 0.055 && $0.hsl.y > 0.08 && $0.hsl.x > 0.42 && $0.hsl.x < 0.78 }
        let light = pick(primary) { $0.lum > 0.52 }
        let accent = pick(light,avoiding:primary,distance:0.075) { $0.chroma > 0.075 && $0.hsl.y > 0.10 && $0.lum > 0.10 && $0.lum < 0.92 }
        var result = Self()
        result.primary = primary.color; result.base = base.color; result.warm = warm.color; result.cool = cool.color; result.light = light.color; result.accent = accent.color
        let averageChroma = chromaSum/Double(total), ratio = Double(colorful)/Double(total)
        result.monochrome = maxChroma < 0.095 || averageChroma < 0.026 || ratio < 0.014 || Double(usable)/Double(total) < 0.006
        let chosen = best ?? average, hsl = chosen.hsl, lum = average.lum
        if !result.monochrome && lum >= 0.16 && chosen.chroma >= 0.055 && hsl.y >= 0.060 && !(lum < 0.30 && (hsl.x < 0.06 || hsl.x > 0.75)) {
            let bright = hsl.z >= 0.62 || lum >= 0.64
            let s = min(1,max(hsl.y*(bright ? 1 : 1.2),0.90))
            let l = bright ? min(0.94,max(hsl.z,0.70)) : min(0.90,max(hsl.z+0.30,0.74))
            result.lyric = color(h:hsl.x,s:s,l:l)
            result.secondary = color(h:(hsl.x+0.08).truncatingRemainder(dividingBy:1),s:min(1,max(s*0.90,0.78)),l:min(0.86,max(0.58,l-0.10)))
            result.highlight = color(h:(hsl.x+0.03).truncatingRemainder(dividingBy:1),s:min(1,max(s*0.82,0.72)),l:min(0.96,max(0.80,l+0.12)))
        }
        return result
    }
    private static func hsl(_ c: SIMD3<Double>) -> SIMD3<Double> {
        let high = max(c.x,max(c.y,c.z)), low = min(c.x,min(c.y,c.z)), l = (high+low)/2, d = high-low
        guard d > 0 else { return SIMD3(0,0,l) }
        let s = l > 0.5 ? d/(2-high-low) : d/(high+low)
        let h = high == c.x ? (c.y-c.z)/d+(c.y < c.z ? 6 : 0) : high == c.y ? (c.z-c.x)/d+2 : (c.x-c.y)/d+4
        return SIMD3(h/6,s,l)
    }
    private static func color(h: Double, s: Double, l: Double) -> SIMD3<Float> {
        let q = l < 0.5 ? l*(1+s) : l+s-l*s, p = 2*l-q
        func hue(_ input: Double) -> Double {
            let t = input < 0 ? input+1 : input > 1 ? input-1 : input
            return t < 1/6 ? p+(q-p)*6*t : t < 0.5 ? q : t < 2/3 ? p+(q-p)*(2/3-t)*6 : p
        }
        let c = s == 0 ? SIMD3(repeating:l) : SIMD3(hue(h+1/3),hue(h),hue(h-1/3))
        return SIMD3<Float>((c*255).rounded(.toNearestOrAwayFromZero))/255
    }
}

/// Retargeting starts from the currently visible colour, including rapid consecutive changes.
struct NativePaletteTransition {
    private(set) var value = NativeCoverPalette()
    private var from = NativeCoverPalette(), target = NativeCoverPalette()
    private var elapsed: Float = 0.75
    mutating func move(to next: NativeCoverPalette) { from = value; target = next; elapsed = 0 }
    mutating func step(_ dt: Float) {
        elapsed = min(0.75,elapsed+max(0,dt)); let t = elapsed/0.75
        value = from.blended(to: target, amount: t*t*(3-2*t))
    }
}
