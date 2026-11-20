import AppKit
import Observation

/// Only visible shelf cards retain thumbnails. Downloads share the normal artwork owner.
@MainActor @Observable final class MineradioQueueArtwork {
    @ObservationIgnored private var identities: [Int: String] = [:]
    private(set) var images: [Int: String] = [:]
    func load(_ store: MusicStore, indices: [Int], enabled: Bool) async {
        let session = store.sessionID
        guard enabled, let zone = store.zone else { images = [:]; identities = [:]; return }
        let selected = Array(Set(indices)).sorted().filter { zone.queue.indices.contains($0) }.prefix(11)
        let keys = Dictionary(uniqueKeysWithValues: selected.map { ($0, "\(session):\(zone.queue[$0].id):\(zone.queue[$0].cover_url ?? "")") })
        images = images.filter { keys[$0.key] != nil && keys[$0.key] == identities[$0.key] }
        identities = keys
        for index in selected {
            guard !Task.isCancelled, session == store.sessionID else { return }
            if images[index] != nil { continue }
            let track = zone.queue[index]
            guard let image = await store.artwork.image(track, store: store), !Task.isCancelled,
                  let bitmap = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { continue }
            let worker = Task.detached(priority: .utility) { () -> String? in
                guard !Task.isCancelled, let context = CGContext(data: nil, width: 160, height: 160, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
                context.interpolationQuality = .medium
                context.draw(bitmap, in: CGRect(x: 0, y: 0, width: 160, height: 160))
                guard let thumb = context.makeImage(), !Task.isCancelled,
                      let jpeg = NSBitmapImageRep(cgImage: thumb).representation(using: .jpeg, properties: [.compressionFactor: 0.72]), jpeg.count < 48 * 1024 else { return nil }
                return "data:image/jpeg;base64," + jpeg.base64EncodedString()
            }
            let data = await withTaskCancellationHandler { await worker.value } onCancel: { worker.cancel() }
            guard !Task.isCancelled, session == store.sessionID, store.zone?.queue.indices.contains(index) == true,
                  store.zone?.queue[index].id == track.id else { return }
            if let data { images[index] = data }
        }
    }
}
