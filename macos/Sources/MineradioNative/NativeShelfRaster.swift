import CoreText
import ImageIO
import Foundation

struct NativeShelfContent: Sendable, Equatable {
    let item: NativeQueueItem
    let current: Bool
    let background: Float
    let accent: SIMD3<Float>
}
struct NativeShelfBitmap: Sendable {
    let content: NativeShelfContent
    let pixels: [UInt8]
}

/// Static card texture only. Movement, hover, selection and pulse stay on the GPU.
enum NativeShelfRaster {
    static let width = 720, height = 360
    static func make(_ content: NativeShelfContent) throws -> NativeShelfBitmap {
        try Task.checkCancellation()
        var pixels = [UInt8](repeating:0,count:width*height*4)
        pixels.withUnsafeMutableBytes { raw in
            guard let ctx = CGContext(data:raw.baseAddress,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else { return }
            func color(_ r: CGFloat,_ g: CGFloat,_ b: CGFloat,_ a: CGFloat) -> CGColor { CGColor(red:r,green:g,blue:b,alpha:a) }
            let rect = CGRect(x:18,y:18,width:684,height:324)
            let path = CGPath(roundedRect:rect,cornerWidth:32,cornerHeight:32,transform:nil)
            ctx.addPath(path); ctx.setFillColor(color(0,0,0,CGFloat(content.background))); ctx.fillPath()
            ctx.saveGState(); ctx.addPath(path); ctx.clip()
            if let gradient = CGGradient(colorsSpace:CGColorSpaceCreateDeviceRGB(),colors:[color(1,1,1,0.1),color(1,1,1,0.018)] as CFArray,locations:[0,1]) {
                ctx.drawLinearGradient(gradient,start:CGPoint(x:0,y:360),end:CGPoint(x:720,y:0),options:[])
            }
            ctx.restoreGState()
            let accent = color(CGFloat(content.accent.x),CGFloat(content.accent.y),CGFloat(content.accent.z),0.72)
            ctx.addPath(path); ctx.setStrokeColor(content.current ? accent : color(1,1,1,0.14)); ctx.setLineWidth(content.current ? 1.8 : 1.1); ctx.strokePath()
            let coverRect = CGRect(x:24,y:22,width:316,height:316)
            ctx.saveGState(); ctx.addPath(CGPath(roundedRect:coverRect,cornerWidth:26,cornerHeight:26,transform:nil)); ctx.clip()
            ctx.setFillColor(color(1,1,1,0.04)); ctx.fill(coverRect)
            if content.item.cover.utf8.count < 65536, let comma = content.item.cover.firstIndex(of:","),
               let data = Data(base64Encoded:String(content.item.cover[content.item.cover.index(after:comma)...])),
               let source = CGImageSourceCreateWithData(data as CFData,nil),
               let image = CGImageSourceCreateThumbnailAtIndex(source,0,[kCGImageSourceCreateThumbnailFromImageAlways:true,kCGImageSourceThumbnailMaxPixelSize:320] as CFDictionary) {
                ctx.interpolationQuality = .high; ctx.draw(image,in:coverRect)
            }
            ctx.restoreGState()
            func text(_ value: String, y: CGFloat, size: CGFloat, bold: Bool, ink: CGColor, lines: Int = 1) {
                let font = CTFontCreateWithName((bold ? "PingFangSC-Semibold" : "PingFangSC-Regular") as CFString,size,nil)
                let attr = NSAttributedString(string:String(value.prefix(500)),attributes:[NSAttributedString.Key(kCTFontAttributeName as String):font,NSAttributedString.Key(kCTForegroundColorAttributeName as String):ink])
                let setter = CTTypesetterCreateWithAttributedString(attr)
                var start = 0
                for row in 0..<lines where start < attr.length {
                    let length = max(1,CTTypesetterSuggestLineBreak(setter,start,324))
                    let line = CTTypesetterCreateLine(setter,CFRange(location:start,length:min(length,attr.length-start)))
                    ctx.textPosition = CGPoint(x:366,y:y-CGFloat(row)*(size+6)); CTLineDraw(line,ctx); start += length
                }
            }
            text(content.current ? "正在播放" : "#\(content.item.index+1)",y:306,size:17,bold:true,ink:content.current ? accent : color(1,1,1,0.92))
            text(content.item.title,y:264,size:30,bold:true,ink:color(1,1,1,0.96),lines:2)
            text(content.item.artist,y:186,size:17,bold:false,ink:color(1,1,1,0.52),lines:2)
            text("点击播放",y:95,size:14,bold:true,ink:accent)
            ctx.setStrokeColor(content.current ? accent : color(1,1,1,0.3)); ctx.setLineWidth(3.5)
            ctx.move(to:CGPoint(x:366,y:40));ctx.addLine(to:CGPoint(x:446,y:40));ctx.strokePath()
        }
        try Task.checkCancellation()
        return NativeShelfBitmap(content:content,pixels:pixels)
    }
}
