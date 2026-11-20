import Foundation
import Observation

@MainActor @Observable
final class DiscoveryStore {
    var cores: [DiscoveredCore] = []
    var searching = false
    var error: String?
    var selectedID: String?
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var scanUntil = Date.distantPast
    func start(_ store: MusicStore) {
        guard task == nil else { return }
        searching = true; scanUntil = Date.now.addingTimeInterval(6)
        task = Task { [weak self, weak store] in
            while !Task.isCancelled {
                guard let result = await store?.discoverySnapshot() else { return }
                guard !Task.isCancelled else { return }
                switch result {
                case .success(let page): self?.cores = page.items; self?.error = nil
                case .failure(let error): self?.error = error.localizedDescription
                }
                self?.searching = self?.cores.isEmpty == true && Date.now < (self?.scanUntil ?? .distantPast)
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
            }
        }
    }
    func stop() { task?.cancel(); task = nil; searching = false }
    func refresh(_ store: MusicStore) { scanUntil = Date.now.addingTimeInterval(6); searching = true; start(store) }
}

extension MusicStore {
    fileprivate func discoverySnapshot() async -> Result<DiscoveryPage, Error> {
        do { return .success(try await runtimeCall("discover").decode(DiscoveryPage.self)) }
        catch { return .failure(error) }
    }
}
