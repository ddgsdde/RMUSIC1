import Foundation

/// One private child per app, no localhost RPC port and no credentials in command arguments.
@MainActor
final class RuntimeClient {
    var onTravelTerminal: ((String)->Void)?
    var onCoreEvent: ((JSONValue)->Void)?
    private var process: Process?
    private var input: FileHandle?
    private var output: FileHandle?
    private var diagnosticOutput: FileHandle?
    private var pending: [String: CheckedContinuation<JSONValue, Error>] = [:]
    private var deadlines: [String: Task<Void, Never>] = [:]
    private let writes = DispatchQueue(label: "com.rmusic.runtime-input", qos: .userInitiated)
    private(set) var generation = UUID()

    isolated deinit {
        output?.readabilityHandler = nil
        diagnosticOutput?.readabilityHandler = nil
        try? input?.close()
        try? output?.close()
        try? diagnosticOutput?.close()
        deadlines.values.forEach { $0.cancel() }
        process?.terminationHandler = nil
    }

    func start() throws {
        guard process == nil else { return }
        let executable = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/rmusic-mac-runtime")
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw RuntimeFailure(message: "找不到原生运行时，请通过 macos/script/build_and_run.sh 构建应用。")
        }
        let p = Process(), stdin = Pipe(), stdout = Pipe(), stderr = Pipe()
        if Bundle.main.object(forInfoDictionaryKey: "CFBundleExecutable") as? String == "RMusicBridge" {
            p.arguments = ["--standalone-bridge"]
        }
        let directory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent(Bundle.main.bundleIdentifier ?? "com.rmusic.mac.development", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        var environment = ProcessInfo.processInfo.environment
        environment["RMUSIC_MAC_DATA"] = directory.path
        if let file = Bundle.main.url(forResource: "development-tools", withExtension: "json"),
           let data = try? Data(contentsOf: file), let paths = try? JSONDecoder().decode([String: String].self, from: data) {
            for (key, path) in paths { environment[key] = path }
        }
        // GUI launch environments may omit /usr/sbin; use trusted system tools first.
        environment["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin:" + (environment["PATH"] ?? "")
        environment["RMUSIC_PLAYBACK_DIAGNOSTICS"] = "1"
        p.executableURL = executable; p.environment = environment
        p.standardInput = stdin; p.standardOutput = stdout; p.standardError = stderr
        let diagnostics = RuntimeDiagnostics()
        stderr.fileHandleForReading.readabilityHandler = { handle in diagnostics.receive(handle.availableData) }
        let epoch = UUID(); generation = epoch
        let reader = RuntimeResponseReader()
        stdout.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let result = reader.read(handle.availableData)
            if case .success(let responses) = result, responses.isEmpty { return }
            Task { @MainActor [weak self] in self?.receive(result, generation: epoch) }
        }
        p.terminationHandler = { [weak self] _ in
            Task { @MainActor [weak self] in self?.failed("原生运行时已退出，请重新连接。", generation: epoch) }
        }
        do { try p.run() }
        catch { stdout.fileHandleForReading.readabilityHandler = nil; stderr.fileHandleForReading.readabilityHandler = nil; throw error }
        process = p; input = stdin.fileHandleForWriting; output = stdout.fileHandleForReading
        diagnosticOutput = stderr.fileHandleForReading
    }

    func call(_ operation: String, _ body: JSONValue = .null) async throws -> JSONValue {
        try start()
        guard pending.count < 32 else { throw RuntimeFailure(message: "请求繁忙，请稍后重试。", status: 429) }
        let id = UUID().uuidString
        var data = try JSONEncoder().encode(RuntimeCommand(id: id, operation: operation, body: body))
        data.append(10)
        guard data.count <= 4 * 1024 * 1024 + 65536 else { throw RuntimeFailure(message: "请求内容过大，尚未发送。") }
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                pending[id] = continuation
                deadlines[id] = Task { [weak self] in
                    let seconds: Int
                    switch operation {
                    case "save_api_file", "import_account_file": seconds = 14_410
                    case "install_plugin": seconds = 95
                    case "request": seconds = ["/recommendations/daily", "/recommendations/similar"].contains(body["path"].text) ? 315 : 60
                    case "connect": seconds = 60
                    default: seconds = 25
                    }
                    do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
                    self?.cancelRequest(id)
                    self?.finish(id, result: .failure(RuntimeFailure(message: "原生请求超时。写操作可能已在 Core 执行，请刷新状态。")))
                }
                send(data, id: id)
            }
        } onCancel: {
            Task { @MainActor [weak self] in self?.cancelRequest(id); self?.finish(id, result: .failure(CancellationError())) }
        }
    }
    private func send(_ data: Data, id: String) {
        guard let input else { finish(id, result: .failure(RuntimeFailure(message: "运行时未连接。"))); return }
        let epoch = generation
        writes.async { [weak self] in
            do { try input.write(contentsOf: data) }
            catch { Task { @MainActor [weak self] in
                guard self?.generation == epoch else { return }
                self?.finish(id, result: .failure(error))
            } }
        }
    }
    private func cancelRequest(_ id: String) {
        guard pending[id] != nil else { return }
        if var data = try? JSONEncoder().encode(RuntimeCommand(id: UUID().uuidString, operation: "cancel", body: .object(["id": .string(id)]))) {
            data.append(10); send(data, id: id)
        }
    }
    private func receive(_ result: Result<[RuntimeResponse], Error>, generation epoch: UUID) {
        guard generation == epoch else { return }
        switch result {
        case .success(let responses):
            for response in responses {
                if response.id == "core-event" {
                    if let event = response.result { onCoreEvent?(event) }
                    continue
                }
                if response.id=="travel-terminal" {
                    if let id=response.result?.string {onTravelTerminal?(id)}
                    continue
                }
                if let message = response.error { finish(response.id, result: .failure(RuntimeFailure(message: message, status: response.status))) }
                else { finish(response.id, result: .success(response.result ?? .null)) }
            }
        case .failure(let error): failed(error.localizedDescription, generation: epoch)
        }
    }
    private func finish(_ id: String, result: Result<JSONValue, Error>) {
        deadlines.removeValue(forKey: id)?.cancel()
        pending.removeValue(forKey: id)?.resume(with: result)
    }
    private func failed(_ message: String, generation epoch: UUID) {
        guard generation == epoch else { return }
        output?.readabilityHandler = nil
        diagnosticOutput?.readabilityHandler = nil
        try? diagnosticOutput?.close(); diagnosticOutput = nil
        try? input?.close() // EOF requests a joined native shutdown.
        process = nil; input = nil; output = nil
        for id in Array(pending.keys) { finish(id, result: .failure(RuntimeFailure(message: message))) }
    }
    func shutdown() async {
        guard process != nil else { return }
        _ = try? await call("quit")
        try? input?.close()
    }
}
