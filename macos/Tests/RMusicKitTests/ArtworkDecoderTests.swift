import CoreGraphics
import Foundation
import ImageIO
import Testing

@testable import RMusicKit

@Test func artworkDownsamplingIsBoundedAndCancelledDecodeDoesNotPublishAnImage() async throws {
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
    "rmusic-artwork-decode-" + UUID().uuidString)
  try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
  defer { try? FileManager.default.removeItem(at: directory) }
  let path = directory.appendingPathComponent("cover.png")
  let context = try #require(
    CGContext(
      data: nil, width: 1000, height: 800, bitsPerComponent: 8, bytesPerRow: 4000,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
  context.setFillColor(CGColor(red: 0.3, green: 0.2, blue: 0.8, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: 1000, height: 800))
  let image = try #require(context.makeImage())
  let destination = try #require(
    CGImageDestinationCreateWithURL(path as CFURL, "public.png" as CFString, 1, nil))
  CGImageDestinationAddImage(destination, image, nil)
  #expect(CGImageDestinationFinalize(destination))
  let thumbnail = try #require(try await ArtworkDecoder.decode(path.path))
  #expect(thumbnail.width == 640 && thumbnail.height == 512)
  let large = directory.appendingPathComponent("valid-6m.image")
  var larger = try Data(contentsOf: path)
  larger.append(Data(repeating: 0, count: 6 * 1024 * 1024 - larger.count))
  try larger.write(to: large)
  #expect(try await ArtworkDecoder.decode(large.path) != nil)
  let cancelled = Task { try await ArtworkDecoder.decode(path.path) }
  cancelled.cancel()
  do {
    _ = try await cancelled.value
    Issue.record("cancelled decode published a thumbnail")
  } catch is CancellationError {} catch { Issue.record(error) }
  let invalid = directory.appendingPathComponent("invalid.image")
  try Data("invalid image".utf8).write(to: invalid)
  #expect(try await ArtworkDecoder.decode(invalid.path) == nil)
}
