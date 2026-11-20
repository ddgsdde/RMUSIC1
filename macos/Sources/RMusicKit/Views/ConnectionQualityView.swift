import SwiftUI

private struct LinkMeasurement: Decodable {
    let mode: String
    let underlay: String?
    let address: String
    let latency_ms: Double
    let down_mbps: Double?
    let end_to_end: Bool?
    var label: String {
        ["local":"本机直连","lan":"局域网直连","vpn":"VPN／专网","private":"私网直连","https":"HTTPS 直连","reverse_proxy":"HTTPS 反代","direct":"HTTP 直连"][mode] ?? "连接方式待识别"
    }
}

struct ConnectionQualityView: View {
    let store: MusicStore
    @Environment(\.scenePhase) private var phase
    @StoredState private var measurement: LinkMeasurement?
    @StoredState private var rate: Double?
    @StoredState private var error: String?
    @StoredState private var running = false
    @StoredState private var request = 0
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label(measurement?.label ?? "当前连接", systemImage: "point.3.connected.trianglepath.dotted")
                Spacer()
                Text(measurement.map { "\(Int($0.latency_ms.rounded())) ms" } ?? (running ? "测量中…" : "延迟未知")).monospacedDigit()
            }.font(.headline)
            Text(measurement?.address ?? store.connectionAddress).font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
            if let value = measurement, value.end_to_end == true {
                Text(value.underlay == "vpn" && value.mode != "vpn" ? "经 VPN · 端到端加密" : "端到端加密").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Button(running && request > 0 ? "测量中…" : "下载测速") { request += 1 }.disabled(running)
                Text(rate.map { String(format: "↓ %.1f Mbps", $0) } ?? "下载 1 MiB · 应用往返延迟").font(.caption).foregroundStyle(.secondary)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.secondary) }
        }
        .task(id: "\(store.sessionID)-\(phase)-\(request)") {
            guard phase == .active else { return }
            var speed = request > 0 && request != lastTest
            if speed { lastTest = request }
            while !Task.isCancelled {
                running = true
                do {
                    let next = try await store.runtimeCall("connection_metrics", .object(["speed": .bool(speed)])).decode(LinkMeasurement.self)
                    try Task.checkCancellation()
                    if measurement?.mode != next.mode || measurement?.address != next.address { rate = nil }
                    measurement = next
                    if let value = next.down_mbps { rate = value }
                    error = nil
                } catch {
                    guard !Task.isCancelled else { return }
                    self.error = error.localizedDescription
                    measurement = nil; rate = nil
                }
                running = false; speed = false
                do { try await Task.sleep(for: .seconds(60)) } catch { return }
            }
        }
    }
    @StoredState private var lastTest = 0
}
