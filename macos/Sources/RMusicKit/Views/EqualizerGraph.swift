import SwiftUI

enum EqualizerResponse {
    typealias Band = PlaybackSettingsDraft.Band
    typealias Section = (Double, Double, Double, Double, Double, Double)

    static func sections(_ band: Band, rate: Int) -> [Section] {
        guard band.enabled, band.frequency >= 10, band.frequency < Double(rate) / 2, band.q >= 0.1 else { return [] }
        let angle = 2 * Double.pi * band.frequency / Double(rate)
        let c = cos(angle), s = sin(angle)
        let a = pow(10, band.gain / 40), sum = a + 1, diff = a - 1
        let count = band.kind.isCut ? band.slope / 12 : 1
        return (0..<count).map { index in
            let q = count > 1 ? band.q / sqrt(2) / cos(.pi * Double(2 * index + 1) / Double(4 * count)) : band.q
            let alpha = s / (2 * q), shelf = 2 * sqrt(a) * alpha
            switch band.kind {
            case .lowShelf:
                return (a*(sum-diff*c+shelf), 2*a*(diff-sum*c), a*(sum-diff*c-shelf), sum+diff*c+shelf, -2*(diff+sum*c), sum+diff*c-shelf)
            case .highShelf:
                return (a*(sum+diff*c+shelf), -2*a*(diff+sum*c), a*(sum+diff*c-shelf), sum-diff*c+shelf, 2*(diff-sum*c), sum-diff*c-shelf)
            case .lowPass: return ((1-c)/2, 1-c, (1-c)/2, 1+alpha, -2*c, 1-alpha)
            case .highPass: return ((1+c)/2, -(1+c), (1+c)/2, 1+alpha, -2*c, 1-alpha)
            case .bandPass: return (alpha, 0, -alpha, 1+alpha, -2*c, 1-alpha)
            case .notch: return (1, -2*c, 1, 1+alpha, -2*c, 1-alpha)
            case .allPass: return (1-alpha, -2*c, 1+alpha, 1+alpha, -2*c, 1-alpha)
            case .peak: return (1+alpha*a, -2*c, 1-alpha*a, 1+alpha/a, -2*c, 1-alpha/a)
            }
        }
    }

    static func at(_ hz: Double, rate: Int, sections: [Section]) -> Double {
        let angle = 2 * Double.pi * hz / Double(rate)
        let c = cos(angle), s = sin(angle), c2 = cos(2*angle), s2 = sin(2*angle)
        func power(_ x: Double, _ y: Double, _ z: Double) -> Double {
            let real = x + y*c + z*c2, imag = y*s + z*s2
            return real*real + imag*imag
        }
        return sections.reduce(0) { dB, b in
            dB + 10 * log10(max(1e-15, power(b.0,b.1,b.2) / max(1e-30,power(b.3,b.4,b.5))))
        }
    }

    static func upperLimit(_ rate: Int) -> Double { max(1000, min(20_000, Double(rate) * 0.495)) }
    static func frequency(_ fraction: CGFloat, high: Double) -> Double { 10 * pow(high / 10, fraction) }
    static func peak(bands: [Band], rate: Int) -> Double {
        let filters = bands.flatMap { sections($0, rate: rate) }
        return max(0,(0...240).map { at(frequency(CGFloat($0)/240,high:upperLimit(rate)),rate:rate,sections:filters) }.max() ?? 0)
    }
}

struct EqualizerGraph: View {
    @Binding var bands: [PlaybackSettingsDraft.Band]
    @Binding var selected: UUID?
    let rate: Int
    let bypassed: Bool
    let add: (Double, Double) -> Void
    @StoredState private var range = 18
    @StoredState private var dragging: UUID?
    @Environment(\.colorScheme) private var scheme
    static var colors: [Color] { [MusicStyle.accent, .purple, .orange, .cyan, .pink, .green] }
    private var graphSurface: Color { scheme == .dark ? .black.opacity(0.22) : .white.opacity(0.5) }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text("综合响应").fontWeight(.semibold)
                Text("· \(Double(rate)/1000, specifier: "%g") kHz 预览").foregroundStyle(.secondary)
                Spacer()
                Picker("图表增益刻度", selection: $range) {
                    ForEach([6,12,18,30],id: \.self) { scale in Text("±\(scale) dB").tag(scale) }
                }.labelsHidden().fixedSize()
            }.font(.caption).padding(.horizontal, 14).padding(.top, 11)
            GeometryReader { proxy in
                let plot = Plot(size: proxy.size, rate: rate, range: range)
                Canvas { ctx, _ in draw(ctx, plot: plot) }
                    .contentShape(Rectangle())
                    .gesture(DragGesture(minimumDistance: 0).onChanged { value in
                        if dragging == nil {
                            guard let index = nearest(value.startLocation, plot: plot) else { return }
                            dragging = bands[index].id
                            selected = dragging
                        }
                        guard let index = bands.firstIndex(where: { $0.id == dragging }) else { return }
                        if abs(value.translation.width) + abs(value.translation.height) > 1 {
                            let value = plot.parameters(value.location)
                            bands[index].frequency = value.0
                            if bands[index].kind.hasGain { bands[index].gain = value.1 }
                        }
                    }.onEnded { _ in dragging = nil })
                    .simultaneousGesture(SpatialTapGesture(count: 2).onEnded { value in
                        guard nearest(value.location, plot: plot) == nil else { return }
                        let values = plot.parameters(value.location)
                        add(values.0, values.1)
                    })
            }.frame(height: 270).accessibilityLabel("均衡器响应图")
            Text("拖动频段调节频率和增益，双击空白添加频段。")
                .font(.caption).foregroundStyle(.secondary).frame(maxWidth:.infinity, alignment:.leading).padding(.horizontal, 14).padding(.bottom, 12)
        }
        .background(graphSurface,in:RoundedRectangle(cornerRadius:16))
        .overlay(RoundedRectangle(cornerRadius:16).strokeBorder(.primary.opacity(0.08)).allowsHitTesting(false))
    }

    private func nearest(_ point: CGPoint, plot: Plot) -> Int? {
        var best: Int?
        var threshold: CGFloat = 17
        for index in bands.indices where bands[index].frequency <= plot.high {
            let anchor = plot.anchor(bands[index])
            let distance = hypot(anchor.x - point.x, anchor.y - point.y)
            if distance < threshold {
                best = index
                threshold = distance
            }
        }
        return best
    }

    private func draw(_ ctx: GraphicsContext, plot: Plot) {
        let light = Color.primary.opacity(0.56), grid = Color.primary.opacity(0.10)
        func line(_ a: CGPoint,_ b: CGPoint,_ color: Color) {
            var path = Path(); path.move(to:a); path.addLine(to:b); ctx.stroke(path,with:.color(color))
        }
        for value in [-range, -range/2, 0, range/2, range] {
            let y = plot.y(Double(value))
            line(CGPoint(x:48,y:y),CGPoint(x:plot.right,y:y),value == 0 ? .primary.opacity(0.30) : grid)
            ctx.draw(Text(value == 0 ? "0 dB" : "\(value > 0 ? "+" : "")\(value)").font(.system(size: 10,design:.monospaced)).foregroundStyle(light),at:CGPoint(x:24,y:y))
        }
        for hz in [10,20,50,100,200,500,1000,2000,5000,10000,20000].map(Double.init) where hz <= plot.high {
            let x = plot.x(hz)
            line(CGPoint(x:x,y:14),CGPoint(x:x,y:plot.bottom),grid)
            ctx.draw(Text(hz >= 1000 ? "\(Int(hz/1000))k" : "\(Int(hz))").font(.system(size: 10,design:.monospaced)).foregroundStyle(light),at:CGPoint(x:x,y:plot.bottom+16))
        }
        func trace(_ filters: [EqualizerResponse.Section]) -> Path {
            var path = Path()
            for i in 0...240 {
                let hz = EqualizerResponse.frequency(CGFloat(i)/240, high:plot.high)
                let point = CGPoint(x:plot.x(hz),y:plot.y(EqualizerResponse.at(hz,rate:rate,sections:filters)))
                if i == 0 { path.move(to:point) } else { path.addLine(to:point) }
            }
            return path
        }
        let composite = trace(bypassed ? [] : bands.flatMap { EqualizerResponse.sections($0,rate:rate) })
        var fill = composite
        fill.addLine(to: CGPoint(x:plot.right,y:plot.y(0)))
        fill.addLine(to: CGPoint(x:48,y:plot.y(0)))
        fill.closeSubpath()
        let accent = MusicStyle.accent
        ctx.fill(fill,with:.linearGradient(Gradient(colors:[accent.opacity(0.23),accent.opacity(0.025)]),startPoint:CGPoint(x:0,y:14),endPoint:CGPoint(x:0,y:plot.bottom)))
        if let index = bands.firstIndex(where: { $0.id == selected }) {
            var band = bands[index]; band.enabled = true
            ctx.stroke(trace(EqualizerResponse.sections(band,rate:rate)),with:.color(Self.colors[index % Self.colors.count].opacity(0.64)),style:StrokeStyle(lineWidth:1,dash:[4,5]))
        }
        ctx.stroke(composite,with:.color(bypassed ? .primary.opacity(0.45) : accent),lineWidth:2)
        for (index, band) in bands.enumerated() where band.frequency <= plot.high {
            let active = band.id == selected, point = plot.anchor(band)
            let radius = CGFloat(active ? 12 : 9), color = Self.colors[index % Self.colors.count].opacity(band.enabled ? 1 : 0.48)
            let circle = Path(ellipseIn:CGRect(x:point.x-radius,y:point.y-radius,width:radius*2,height:radius*2))
            ctx.fill(circle,with:.color(active ? color : Color(.windowBackgroundColor)))
            ctx.stroke(circle,with:.color(color),lineWidth:1.5)
            let label = active ? (index % Self.colors.count == 0 ? MusicStyle.onAccent : .black.opacity(0.86)) : .primary
            ctx.draw(Text("\(index+1)").font(.system(size:10,weight:.semibold,design:.monospaced)).foregroundStyle(label),at:point)
        }
    }

    private struct Plot {
        let size: CGSize
        let rate: Int
        let range: Int
        var right: CGFloat { max(49,size.width-18) }
        var bottom: CGFloat { max(15,size.height-32) }
        var high: Double { EqualizerResponse.upperLimit(rate) }
        func x(_ hz:Double) -> CGFloat { 48 + max(0,min(1,log(max(10,hz)/10)/log(high/10))) * (right-48) }
        func y(_ value:Double) -> CGFloat {
            let bound = Double(range)
            return 14 + (bound - max(-bound,min(bound,value))) / (2*bound) * (bottom-14)
        }
        func anchor(_ band: PlaybackSettingsDraft.Band) -> CGPoint { CGPoint(x:x(band.frequency),y:y(band.kind.hasGain ? band.gain : 0)) }
        func parameters(_ point:CGPoint) -> (Double,Double) {
            let fraction = max(0,min(1,(point.x-48)/(right-48)))
            let db = Double(range) * (1 - 2 * (point.y-14)/(bottom-14))
            return (EqualizerResponse.frequency(fraction,high:high).rounded(), (max(-30,min(30,db))*10).rounded()/10)
        }
    }
}
