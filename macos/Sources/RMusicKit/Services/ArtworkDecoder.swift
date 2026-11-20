import Foundation
import ImageIO

/// Decode bounded thumbnails away from the UI actor; cancellation keeps its worker owned.
enum ArtworkDecoder {
  static func decode(_ path: String) async throws -> CGImage? {
    let worker = Task.detached(priority: .utility) { try thumbnail(path) }
    return try await withTaskCancellationHandler {
      try await worker.value
    } onCancel: {
      worker.cancel()
    }
  }
  private static func thumbnail(_ path: String) throws -> CGImage? {
    try Task.checkCancellation()
    let size =
      (try FileManager.default.attributesOfItem(atPath: path)[.size] as? NSNumber)?.uint64Value ?? 0
    guard size > 0, size <= 20 * 1024 * 1024 else { return nil }
    return try autoreleasepool {
      guard
        let source = CGImageSourceCreateWithURL(
          URL(fileURLWithPath: path) as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary),
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
        let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
        let height = properties[kCGImagePropertyPixelHeight] as? NSNumber,
        width.doubleValue > 0, height.doubleValue > 0,
        width.doubleValue * height.doubleValue <= 32_000_000
      else { return nil }
      try Task.checkCancellation()
      let bitmap = CGImageSourceCreateThumbnailAtIndex(
        source, 0,
        [
          kCGImageSourceCreateThumbnailFromImageAlways: true,
          kCGImageSourceThumbnailMaxPixelSize: 640, kCGImageSourceShouldCacheImmediately: true,
        ] as CFDictionary)
      try Task.checkCancellation()
      return bitmap
    }
  }
}
