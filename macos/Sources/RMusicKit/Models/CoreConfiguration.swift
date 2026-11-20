import Foundation

/// API units stay at this boundary; the form presents memory in MiB.
enum CoreConfiguration {
    static let statistics: [ConfigurationField] = [.init(key: "statistics_timezone", title: "统计时区 · 默认上海", kind: .choice(Dictionary(uniqueKeysWithValues: (Set(TimeZone.knownTimeZoneIdentifiers + ["Asia/Shanghai", "UTC"])).map { ($0, $0 == "Asia/Shanghai" ? "上海 · Asia/Shanghai" : $0) })))]
    static let resources: [ConfigurationField] = [
        .init(key: "mobile_opus_enabled", title: "允许移动端 Opus 省流转码", kind: .toggle),
        .init(key: "playback_cache_mode", title: "播放缓存策略", kind: .choice(["full": "补齐完整缓存", "partial": "仅保存已读取部分"])),
        .init(key: "managed_memory_bytes", title: "受控内存预算 · MiB", kind: .number(33_554_432...137_438_953_472), divisor: 1_048_576, integral: true),
        .init(key: "background_concurrency", title: "后台总并发", kind: .number(1...16), integral: true),
        .init(key: "analysis_threads", title: "单个分析线程", kind: .number(1...16), integral: true),
        .init(key: "song_fetch_concurrency", title: "同时拉取歌曲数量", kind: .number(1...16), integral: true),
        .init(key: "cache_limit_bytes", title: "音频缓存预算 · GiB", kind: .number(1...112_589_990_684_262), divisor: 1_073_741_824)
    ]
    static let requests: [ConfigurationField] = [
        ("resolve_ms", "播放地址解析"), ("search_ms", "搜索"), ("lyrics_ms", "歌词"),
        ("playlists_ms", "歌单读取与同步"), ("recommendations_ms", "推荐"),
        ("metadata_ms", "资料补全"), ("comments_ms", "乐评与评论")
    ].map { .init(key: "provider_request_limits.\($0.0)", title: "\($0.1)最小间隔 · ms", kind: .number(50...60_000), integral: true) } + [
        .init(key: "provider_request_limits.cooldown_seconds", title: "上游限流冷却 · 秒", kind: .number(1...3600), integral: true)
    ]
    static let catalogScrape: [ConfigurationField] = [
        .init(key: "catalog_scrape_enabled", title: "后台刮削专辑与艺术家", kind: .toggle),
        .init(key: "catalog_scrape_concurrency", title: "刮削并发 · 默认1", kind: .number(1...4), integral: true)
    ]
    static let general: [ConfigurationField] = [
        .init(key: "name", title: "Core 名称", kind: .text),
        .init(key: "metadata_enabled", title: "自动补全资料", kind: .toggle),
        .init(key: "low_performance", title: "低性能模式", kind: .toggle),
        .init(key: "stream_quality", title: "默认在线音质", kind: .choice([
            "best": "最佳可用", "native": "原始", "MP3_128": "标准 · MP3 128", "MP3_320": "高音质 · MP3 320", "FLAC": "无损 · FLAC", "HIRES": "Hi-Res", "MASTER": "母带"
        ]))
    ]
}
enum JobPresentation {
    static func title(_ value: String) -> String {
        ["artwork_check": "检查图片", "artwork_refresh": "刷新图片", "catalog_scrape": "专辑与艺术家刮削", "cache_repair": "补齐所有缺失", "library_files": "曲库文件管理", "scan": "曲库扫描", "metadata": "歌曲资料补全", "loudness": "响度分析", "cache_fill": "完整缓存补齐", "cache_metadata": "缓存资料整理", "download": "音乐文件下载", "playlist_sync": "歌单同步", "playlist_directory": "云端歌单目录", "playlist_page": "云端歌单曲目刷新"][value] ?? value
    }
    static func status(_ value: String) -> String {
        ["pending": "排队中", "running": "正在执行", "interrupted": "等待恢复", "waiting": "等待资源", "paused": "已暂停", "cancelled": "已取消", "completed": "已完成", "failed": "失败", "unable": "无法分析", "idle": "空闲", "waiting_provider": "等待音源", "error": "出错"][value] ?? value
    }
    static func ended(_ status: String) -> Bool { ["completed", "cancelled", "failed", "unable", "error"].contains(status) }
}
