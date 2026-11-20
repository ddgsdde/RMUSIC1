import AppKit
import Foundation
import ImageIO
import Observation
import OSLog

/// Shared requests have explicit consumers. Only the final consumer cancels a download.
@MainActor @Observable final class ArtworkStore {
  private struct Job {
    let id: UUID
    let task: Task<Void, Never>
    var consumers: [UUID: CheckedContinuation<NSImage?, Never>]
    var retirement: Task<Void, Never>?
  }
  private final class ImageEntry: NSObject { let image: NSImage; let expires = Date().addingTimeInterval(60); init(_ image: NSImage) { self.image = image } }
  private static let log = Logger(subsystem: "com.rmusic.mac", category: "Artwork")
  var revision = UUID()
  @ObservationIgnored private var revalidateAfter = 0.0
  @ObservationIgnored private var failures: [String: Date] = [:]
  @ObservationIgnored private let cache = NSCache<NSString, ImageEntry>()
  @ObservationIgnored private var jobs: [String: Job] = [:]
  @ObservationIgnored private var active = 0
  var pendingCount: Int { jobs.count }
  init() {
    cache.countLimit = 80
    cache.totalCostLimit = 64 * 1024 * 1024
  }
  func sourceKey(_ track: Track?) -> String {
    guard let track else { return "" }
    return track.cover_url.flatMap { $0.isEmpty ? nil : $0 } ?? "track:\(track.id)"
  }
  private func key(_ track: Track, epoch: UUID) -> String {
    "\(epoch):\(sourceKey(track))"
  }
  func image(_ track: Track, store: MusicStore, resolveMissing: Bool = false) async -> NSImage? {
    guard !Task.isCancelled,
      track.cover_url?.isEmpty == false || (resolveMissing && !track.id.isEmpty)
    else { return nil }
    let epoch = store.sessionID
    let key = key(track, epoch: epoch)
    let consumer = UUID()
    if let entry = cache.object(forKey: key as NSString), entry.expires > Date() { return entry.image }
    if let until = failures[key], until > Date() { return nil }
    while jobs[key] == nil && jobs.count >= 128 {
      do {
        try await Task.sleep(for: .milliseconds(30))
        try Task.checkCancellation()
      } catch { return nil }
      guard epoch == store.sessionID else { return nil }
    }
    let result: NSImage? = await withTaskCancellationHandler {
      await withCheckedContinuation { (continuation: CheckedContinuation<NSImage?, Never>) in
        guard !Task.isCancelled else {
          continuation.resume(returning: nil)
          return
        }
        if jobs[key] == nil {
          guard jobs.count < 128 else {
            continuation.resume(returning: nil)
            return
          }
          let jobID = UUID()
          let task = Task { [weak self, weak store] in
            guard let self, let store else { return }
            let image = await fetch(track, store: store, epoch: epoch, key: key)
            finish(key: key, jobID: jobID, image: image)
          }
          jobs[key] = Job(id: jobID, task: task, consumers: [:])
        }
        jobs[key]?.retirement?.cancel()
        jobs[key]?.retirement = nil
        jobs[key]?.consumers[consumer] = continuation
      }
    } onCancel: {
      Task { @MainActor [weak self, weak store] in
        self?.release(key: key, consumer: consumer, allowRejoin: store?.outputs.roaming == true)
      }
    }
    guard !Task.isCancelled, epoch == store.sessionID else { return nil }
    return result
  }
  private func fetch(_ track: Track, store: MusicStore, epoch: UUID, key: String) async -> NSImage?
  {
    while active >= 6 {
      do {
        try await Task.sleep(for: .milliseconds(30))
        try Task.checkCancellation()
      } catch { return nil }
      guard epoch == store.sessionID else { return nil }
    }
    guard !Task.isCancelled else { return nil }
    active += 1
    defer { active -= 1 }
    let started = ContinuousClock.now
    let safeID = track.cover_url?.split(separator: "/").last.map(String.init).flatMap { value in value.count == 64 && value.allSatisfy(\.isHexDigit) ? value : nil } ?? "external"
    var reload = false
    for attempt in 0..<2 {
      do {
        let result = try await store.runtimeCall("artwork", .object(["track_id": .string(track.id), "cover_url": .string(track.cover_url ?? ""), "reload": .bool(reload), "revalidate": .number(revalidateAfter)]))
        try Task.checkCancellation()
        guard epoch == store.sessionID, let path = result["path"].string else { return nil }
        let decoded: CGImage?
        do { decoded = try await ArtworkDecoder.decode(path) }
        catch is CancellationError { throw CancellationError() }
        catch { decoded = nil }
        guard let bitmap = decoded else { Self.log.notice("image=\(safeID, privacy: .public) stage=decode attempt=\(attempt) failed"); if attempt == 0 { reload = true; continue }; return nil }
        try Task.checkCancellation()
        guard epoch == store.sessionID else { return nil }
        let image = NSImage(cgImage: bitmap, size: NSSize(width: bitmap.width, height: bitmap.height))
        cache.setObject(ImageEntry(image), forKey: key as NSString, cost: bitmap.bytesPerRow * bitmap.height)
        return image
      } catch {
        let status = (error as? RuntimeFailure)?.status ?? 0
        Self.log.notice("image=\(safeID, privacy: .public) stage=runtime status=\(status) cancelled=\(Task.isCancelled) attempt=\(attempt) elapsed=\(String(describing: started.duration(to: .now)), privacy: .public)")
        guard !Task.isCancelled, epoch == store.sessionID, attempt == 0,
          let status = (error as? RuntimeFailure)?.status, [408, 429, 500, 502, 503, 504].contains(status)
        else { return nil }
        do { try await Task.sleep(for: .milliseconds(600)) } catch { return nil }
      }
    }
    return nil
  }

  private func finish(key: String, jobID: UUID, image: NSImage?) {
    guard let job = jobs[key], job.id == jobID else { return }
    jobs.removeValue(forKey: key)
    job.retirement?.cancel()
    if image == nil { failures = failures.filter { $0.value > Date() }; if failures.count >= 128 { failures.removeAll() }; failures[key] = Date().addingTimeInterval(15) }
    job.consumers.values.forEach { $0.resume(returning: image) }
  }
  private func release(key: String, consumer: UUID, allowRejoin: Bool) {
    guard var job = jobs[key], let continuation = job.consumers.removeValue(forKey: consumer) else {
      return
    }
    if job.consumers.isEmpty {
      if allowRejoin {
        // SwiftUI may recreate an image as a panel or page finishes its transition.
        let id = job.id
        job.retirement = Task { [weak self] in
          do { try await Task.sleep(for: .milliseconds(350)) } catch { return }
          guard let self, let current = jobs[key], current.id == id, current.consumers.isEmpty else { return }
          jobs.removeValue(forKey: key)
          current.task.cancel()
        }
        jobs[key] = job
      } else {
        jobs.removeValue(forKey: key)
        job.task.cancel()
      }
    } else {
      jobs[key] = job
    }
    continuation.resume(returning: nil)
  }
  func retry(_ track: Track, store: MusicStore) {
    let key = key(track, epoch: store.sessionID)
    failures.removeValue(forKey: key)
    cache.removeObject(forKey: key as NSString)
  }
  func reset(revalidate: Bool = false) {
    revalidateAfter = revalidate ? Date.now.timeIntervalSince1970 : 0
    revision = UUID(); failures.removeAll()
    let old = jobs
    jobs = [:]
    cache.removeAllObjects()
    for job in old.values {
      job.retirement?.cancel()
      job.task.cancel()
      job.consumers.values.forEach { $0.resume(returning: nil) }
    }
  }
}
