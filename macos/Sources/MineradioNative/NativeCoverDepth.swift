import Foundation

// Mineradio d43de565, GPL-3.0: same luminance, radius-4 blur, Sobel and center mask.
enum NativeCoverDepth {
    static func make(_ pixels: [UInt8], width w: Int, height h: Int) throws -> [UInt8] {
        guard w > 1, h > 1, w <= 256, h <= 256, pixels.count == w*h*4 else { return [] }
        let n = w*h
        var lum = [Float](repeating:0,count:n), temp = lum, blur = lum
        for i in 0..<n { lum[i] = (Float(pixels[i*4])*0.299+Float(pixels[i*4+1])*0.587+Float(pixels[i*4+2])*0.114)/255 }
        for y in 0..<h {
            try Task.checkCancellation()
            var sum: Double = 0
            for x in -4...4 { sum += Double(lum[y*w+max(0,min(w-1,x))]) }
            for x in 0..<w { temp[y*w+x] = Float(sum/9); sum += Double(lum[y*w+min(w-1,x+5)])-Double(lum[y*w+max(0,x-4)]) }
        }
        for x in 0..<w {
            try Task.checkCancellation()
            var sum: Double = 0
            for y in -4...4 { sum += Double(temp[max(0,min(h-1,y))*w+x]) }
            for y in 0..<h { blur[y*w+x] = Float(sum/9); sum += Double(temp[min(h-1,y+5)*w+x])-Double(temp[max(0,y-4)*w+x]) }
        }
        var out = [UInt8](repeating:0,count:n*4)
        for y in 0..<h {
            try Task.checkCancellation()
            for x in 0..<w {
                let i = y*w+x
                var edge: Float = 0
                if x > 0 && x < w-1 && y > 0 && y < h-1 {
                    let a = blur[i-w-1], b = blur[i-w], c = blur[i-w+1], d = blur[i-1], f = blur[i+1], g = blur[i+w-1], j = blur[i+w], k = blur[i+w+1]
                    let gx = -a-2*d-g+c+2*f+k, gy = -a-2*b-c+g+2*j+k
                    edge = min(1,sqrt(gx*gx+gy*gy)*1.4)
                }
                let cx = (Float(x)/Float(w-1)-0.5)*2, cy = (Float(y)/Float(h-1)-0.5)*2
                let center = 1-min(1,sqrt(cx*cx+cy*cy)*0.75), depth = min(1,blur[i]*0.45+center*0.55)
                out[i*4] = UInt8((max(0,depth)*255).rounded()); out[i*4+1] = UInt8((edge*255).rounded())
                out[i*4+2] = UInt8((min(1,depth*0.6+edge*0.5)*255).rounded()); out[i*4+3] = UInt8((max(0,min(1,lum[i]))*255).rounded())
            }
        }
        return out
    }
}
