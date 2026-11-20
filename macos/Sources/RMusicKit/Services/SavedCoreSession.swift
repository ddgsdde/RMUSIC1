import Foundation

struct SavedCoreSession {
    private struct Entry: Codable {
        let core: String
        let username: String
        let token: String
    }

    let directory: URL

    static var current: Self {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let bundle = Bundle.main.bundleIdentifier ?? "com.rmusic.mac.development"
        return Self(directory: support.appendingPathComponent(bundle).appendingPathComponent("Session"))
    }

    private var file: URL { directory.appendingPathComponent("core.json") }

    func read(core: String, username: String) -> String? {
        guard let data = try? Data(contentsOf: file),
              let entry = try? JSONDecoder().decode(Entry.self, from: data),
              entry.core == core, entry.username == username.lowercased() else { return nil }
        return entry.token
    }

    func save(_ token: String, core: String, username: String) throws {
        let files = FileManager.default
        try files.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let entry = Entry(core: core, username: username.lowercased(), token: token)
        try JSONEncoder().encode(entry).write(to: file, options: .atomic)
        try files.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
    }

    func remove() {
        try? FileManager.default.removeItem(at: file)
    }

}
