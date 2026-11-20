import Foundation

struct SignalStage: Identifiable {
    let name: String
    let icon: String
    let status: String
    let detail: String
    let processed: Bool
    var id: String { name }
}

/// A report belongs to both a track and a playback instance; stale reports never describe a new song.
struct SignalPresentation {
    let stages: [SignalStage]
    let processing: [String]
    let hasReport: Bool
    let resolution: String
    init(zone: Zone?, endpoint: JSONValue, info: JSONValue) {
        let track = zone?.track
        let playback = endpoint["playback"]
        let matches = track != nil && playback["track_id"].string == track?.id &&
            (zone?.playback_id == nil || playback["core_playback_id"].string == zone?.playback_id)
        let path = matches ? playback["signal_path"] : .null
        hasReport = path["sample_rate"].integer > 0
        processing = hasReport ? path["processing"].array.map(\.text) : []
        let source = info["track_id"].string == track?.id ? info : .null
        let format = source["format"].string ?? track?.format ?? ""
        let rate = source["sample_rate"].integer > 0 ? source["sample_rate"].integer : track?.sample_rate ?? 0
        let bits = source["bit_depth"].integer > 0 ? source["bit_depth"].integer : track?.bit_depth ?? 0
        resolution = Self.resolution(path: path, sourceFormat: format, rate: rate, bits: bits)
        let lossy = ["mp3", "aac", "ogg", "opus"].contains(format.lowercased())
        let remote = processing.first { $0.hasPrefix("Remote FLAC transport:") }
        let coreStream = processing.first { $0.hasPrefix("Core stream conversion:") }
        let nativeCore = processing.first { $0.hasPrefix("Core PCM conversion:") }
        let pcm = processing.contains("Core PCM f32le"), aac = processing.contains("WebCodecs AAC decode")
        var nodes = [SignalStage(name: "音源", icon: "opticaldisc", status: lossy ? "有损音源" : source["verified"].boolean ? "已校验" : "待校验", detail: Self.format(format, rate, bits), processed: lossy),
                     SignalStage(name: "传输", icon: "network", status: remote != nil ? "FLAC 无损编码" : aac ? "有损转换" : (pcm || nativeCore != nil) ? "Core 转换" : hasReport ? "原格式" : "待报告", detail: remote.map { ($0.components(separatedBy: ";").first ?? $0).replacingOccurrences(of: "Remote FLAC transport:", with: "HTTPS FLAC：") } ?? nativeCore ?? (aac ? "AAC → PCM" : pcm ? "Core PCM f32le" : hasReport ? "Core → 播放端" : "等待本次播放报告"), processed: pcm || aac || nativeCore != nil)]
        if let coreStream { nodes.append(SignalStage(name: "Core 格式转换", icon: "arrow.trianglehead.2.clockwise.rotate.90", status: format == "dsf" || format == "dff" ? "DSD → PCM" : "已应用", detail: coreStream.replacingOccurrences(of: "Core stream conversion:", with: "Core："), processed: true)) }
        let dsd = processing.first { $0.hasPrefix("DSD PCM decode") }
        nodes.append(SignalStage(name: "解码", icon: "waveform", status: dsd != nil ? "DSD → PCM" : hasReport ? "已报告" : "等待播放端", detail: dsd.map { value in
            value.replacingOccurrences(of: "DSD PCM decode (device):", with: "播放端解码：").replacingOccurrences(of: "DSD PCM decode (core):", with: "Core 解码：")
        } ?? processing.first { $0.localizedCaseInsensitiveContains("decode") || $0.contains("解码") } ?? (hasReport ? "PCM 解码" : "尚无本次播放报告"), processed: dsd != nil || pcm || aac))
        if let conversion = processing.first(where: { $0.hasPrefix("FFmpeg rate conversion") || $0.hasPrefix("FFmpeg channel conversion") }) {
            nodes.append(SignalStage(name: "播放端转换", icon: "arrow.trianglehead.2.clockwise.rotate.90", status: "已应用", detail: conversion.replacingOccurrences(of: "FFmpeg rate conversion", with: "采样率").replacingOccurrences(of: "FFmpeg channel conversion", with: "声道"), processed: true))
        }
        let settings = PlaybackSettingsDraft(zone)
        let portable = zone?.id.hasPrefix("travel-") == true
        let coreDSP = processing.contains { $0.hasPrefix("Core DSP") }
        let clientDSP = processing.contains { $0.hasPrefix("DSP f64") || $0 == "Shared DSP f64" }
        let processor = coreDSP ? "Core" : "播放设备"
        if !portable && settings.enabled && (coreDSP || clientDSP) {
            let processingRate = Double(path["sample_rate"].integer)
            let bands = settings.allEQ.filter { !$0.bypass }.flatMap(\.bands).filter {
                $0.enabled && $0.frequency < processingRate / 2 && (!$0.kind.hasGain || $0.gain != 0)
            }.count
            if bands > 0 {
                nodes.append(SignalStage(name: "参数均衡", icon: "slider.horizontal.3", status: "已应用", detail: "\(processor) · \(bands) 个有效频段", processed: true))
            }
            if settings.preamp != 0 || settings.balance != 0 {
                let gain = settings.preamp == 0 ? nil : "前级 \(settings.preamp.formatted()) dB"
                let balance = settings.balance == 0 ? nil : "\(settings.balance < 0 ? "左" : "右")声道 \((abs(settings.balance) * 100).formatted())%"
                nodes.append(SignalStage(name: "增益与余量", icon: "gauge.with.dots.needle.50percent", status: "已应用", detail: [processor, gain, balance].compactMap { $0 }.joined(separator: " · "), processed: true))
            }
        }
        if (settings.enabled || portable) && settings.normalize && (coreDSP || clientDSP) {
            let coreConfirmed = processing.contains("Core loudness applied")
            let reportedApplied = playback["meter"]["normalization_applied"].boolean || coreConfirmed
            let pending = !reportedApplied && track?.integrated_lufs == nil
            let target = "目标 \(settings.target.formatted()) LUFS"
            let measured = track?.integrated_lufs.map { "实测 \($0.formatted(.number.precision(.fractionLength(1)))) LUFS" }
            let detail = [processor, target, pending ? nil : measured].compactMap { $0 }.joined(separator: " · ")
            let applied = reportedApplied || clientDSP && !processing.contains("Normalization pending measured LUFS") && !pending
            nodes.append(SignalStage(name: "响度均衡", icon: "speaker.wave.3", status: pending ? "待分析" : applied ? "已应用" : "已分析", detail: detail, processed: applied))
        }
        if (settings.fadeIn > 0 || settings.fadeOut > 0) && (coreDSP || processing.contains("Fade envelope") || processing.contains("Shared fade envelope")) {
            let head = settings.fadeIn > 0 ? "曲首 \(settings.fadeIn) ms" : nil
            let tail = settings.fadeOut > 0 ? "曲尾 \(settings.fadeOut) ms" : nil
            nodes.append(SignalStage(name: "淡入淡出", icon: "waveform.path", status: "已应用", detail: [processor, head, tail].compactMap { $0 }.joined(separator: " · "), processed: true))
        }
        nodes.append(SignalStage(name: "输出", icon: "hifispeaker", status: hasReport ? path["exclusive"].boolean ? "独占输出" : "系统混音" : "待报告", detail: hasReport ? "\(path["output"].string ?? endpoint["name"].text)\n\(Self.format("驱动 PCM 有效精度", path["sample_rate"].integer, path["bit_depth"].integer)) · \(path["channels"].integer) 声道" : zone?.name ?? "选择声音设备", processed: false))
        stages = nodes
    }
    static func resolution(path: JSONValue, sourceFormat: String, rate: Int, bits: Int) -> String {
        guard path["sample_rate"].integer > 0 else { return "" }
        if ["dsd_native", "dsd_dop"].contains(path["sample_type"].text) {
            return rate > 0 ? "DSD · \((Double(rate) / 1_000_000).formatted()) MHz" : "DSD"
        }
        let dsd = ["dsf", "dff", "dsd"].contains(sourceFormat.lowercased())
        var rates = [path["sample_rate"].integer, path["mixer_sample_rate"].integer, dsd ? 0 : rate]
        var depths = [path["bit_depth"].integer, dsd ? 0 : bits]
        let transportPattern = /^(?:Remote FLAC transport|Core (?:FLAC transport|PCM conversion)): (\d+) Hz \/ (\d+) bit/
        for stage in path["processing"].array {
            if let format = stage.text.firstMatch(of: transportPattern) {
                if let rate = Int(format.1) { rates.append(rate) }
                if let bits = Int(format.2) { depths.append(bits) }
            }
        }
        let minimumRate = rates.filter { $0 > 0 }.min() ?? 0
        let minimumBits = depths.filter { $0 > 0 }.min()
        return [minimumBits.map { "\($0) bit" }, "\((Double(minimumRate) / 1000).formatted()) kHz"].compactMap { $0 }.joined(separator: " · ")
    }
    private static func format(_ name: String, _ rate: Int, _ bits: Int) -> String {
        [name.isEmpty || name == "unknown" ? "格式未报告" : name.uppercased(), bits > 0 ? "\(bits) bit" : "位深未报告", rate > 0 ? "\((Double(rate) / 1000).formatted()) kHz" : "采样率未报告"].joined(separator: " · ")
    }
}
