import Foundation

func listeningHistoryDate(milliseconds: Double, timezone: String) -> String {
    guard milliseconds.isFinite else { return "—" }
    let style = Date.FormatStyle(date: .numeric, time: .standard, timeZone: TimeZone(identifier: timezone) ?? .gmt)
    return Date(timeIntervalSince1970: milliseconds / 1000).formatted(style)
}

func listeningFinishReason(_ reason: String) -> String {
    ["ended": "播放到结尾", "next": "下一首", "previous": "上一首", "stop": "主动停止",
     "replaced": "切换曲目", "interrupted": "播放中断", "disconnect": "设备断开",
     "delete": "音区移除", "replay": "重新播放"][reason] ?? (reason.isEmpty ? "会话未结束" : reason)
}
