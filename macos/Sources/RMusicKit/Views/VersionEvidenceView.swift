import SwiftUI

struct VersionEvidenceView: View {
    let evidence: JSONValue
    let identity: JSONValue
    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            let storage = evidence["storage"].text
            Text(storage == "core_cache" ? "Core 缓存副本" : storage == "local_file" ? "本地文件" : "平台音源").font(.caption.weight(.semibold))
            Text("来源 · \(evidence["source"].text)").font(.caption).foregroundStyle(.secondary)
            if let label = evidence["provider_declared"]["label"].string { Text("平台声明 · \(label)").font(.caption) }
            let file = evidence["file_record"]
            if let format = file["format"].string { Text("\(file["verified"].boolean ? "文件记录" : "来源记录") · \(format.uppercased())\(file["sample_rate"].number > 0 ? " · \(Int(file["sample_rate"].number)) Hz" : "")\(file["bit_depth"].number > 0 ? " · \(Int(file["bit_depth"].number)) bit" : "")").font(.caption) }
            if file["channels"].number > 0 { Text("\(file["channels"].integer) 声道 · \(file["duration"].number.formatted(.number.precision(.fractionLength(1)))) 秒").font(.caption).foregroundStyle(.secondary) }
            ForEach(["work", "recording", "release"], id: \.self) { kind in
                ForEach(Array(identity["entities"].array.filter { $0["kind"].text == kind }.enumerated()), id: \.offset) { _, entity in
                    HStack { Text(["work": "作品", "recording": "录音", "release": "发行"][kind] ?? kind); Text(entity["title"].text); Text(entity["basis"].text == "manual" ? "人工确认" : entity["namespace"].text).foregroundStyle(.secondary) }.font(.caption)
                }
            }
            Text("实际输出以当前音区链路报告为准；版本标签不代表音质认证。").font(.caption2).foregroundStyle(.secondary)
        }.textSelection(.enabled)
    }
}
