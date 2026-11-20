import Foundation

/// Shared, pinned visual resources. Playback and application code do not live in this package.
public enum RMusicVisualAssets {
    public static var directory: URL? {
        if let directory = Bundle.main.resourceURL?.appendingPathComponent("Mineradio"), FileManager.default.fileExists(atPath: directory.appendingPathComponent("index.html").path) { return directory }
        return Bundle.module.url(forResource: "Mineradio", withExtension: nil)
    }
}
