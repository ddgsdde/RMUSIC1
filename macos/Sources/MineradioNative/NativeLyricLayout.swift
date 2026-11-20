import Foundation

/// Virtual slots from the pinned 08-lyrics-display-modes.js. Translations reserve
/// their own space rather than being painted into the gap between primary rows.
struct NativeLyricLayout {
    let primary: [Float]
    let translationGap: Float
    let lineStep: Float
    let translationStep: Float
    let maximumSlot: Float
    let envelopeSlot: Float

    init(lines: [NativeLyricLine] = [], parameters p: NativeParameters = NativeParameters("{}")) {
        func clamp(_ value: Float, _ low: Float, _ high: Float) -> Float { min(high,max(low,value)) }
        let spread = clamp(p.number("lyricContextSpread",1.96),0.60,2.40)
        let gap = clamp(p.number("lyricTranslationGap",0.92),0.28,2.20)
        let scale = clamp(p.number("lyricTranslationScale",0.65),0.46,1.12)
        let active = p.raw["lyricTranslationMode"] as? String != "off"
        translationGap = clamp(0.98+(gap-0.28)*0.36+max(0,scale-0.66)*0.12,0.92,2.20)
        let slot = clamp(translationGap+0.82+scale*0.14,1.78,2.88)
        var prefix: [Float] = [0]
        prefix.reserveCapacity(lines.count+1)
        for index in lines.indices {
            let translated = lines[index].translation?.isEmpty == false || (index+1 < lines.count && lines[index+1].translation?.isEmpty == false)
            let step: Float = !active ? 1 : translated ? slot : clamp(1.04+(spread-1)*0.10,0.96,1.24)
            prefix.append(prefix.last!+step)
        }
        primary = prefix
        maximumSlot = zip(prefix,prefix.dropFirst()).map { $1-$0 }.max() ?? 1
        // A translation toggle changes spacing, not the projection's font size.
        envelopeSlot = max(slot,maximumSlot)
        lineStep = clamp(0.60*clamp(1+(spread-1)*0.32,0.86,1.45)*(active ? 1.06 : 1)*p.number("lyricLineHeight",1),0.22,0.94)
        translationStep = clamp(0.60*(active ? 1.04 : 1)*p.number("lyricLineHeight",1),0.20,0.78)
    }
    func position(_ line: Float) -> Float {
        guard primary.count > 1 else { return line }
        let index = max(0,min(primary.count-2,Int(line)))
        return primary[index]+(primary[index+1]-primary[index])*(line-Float(index))
    }
    func y(index: Int, translation: Bool, displayed: Float) -> Float {
        (position(displayed)-position(Float(index)))*lineStep-(translation ? translationGap*translationStep : 0)
    }
}
