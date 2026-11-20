import SwiftUI

/// Palette, workspace density and listening composition are independent preferences.
enum WorkspaceSkin: String, CaseIterable, Identifiable {
    case studio, editorial, compact, mineradio
    var id: String { rawValue }
    var name: String { switch self { case .studio: "柔和工作室"; case .editorial: "唱片杂志"; case .compact: "精简控制台"; case .mineradio: "Mineradio 沉浸" } }
    var sidebarWidth: CGFloat { switch self { case .studio: 216; case .editorial: 236; case .compact: 192; case .mineradio: 212 } }
    var rowPadding: CGFloat { self == .compact ? 9 : 12 }
    var cornerRadius: CGFloat { switch self { case .studio: 18; case .editorial: 6; case .compact: 10; case .mineradio: 18 } }
}
enum ListeningTheme: String, CaseIterable, Identifiable {
    case silk, midnight, paper, aurora, mineradio
    var id: String { rawValue }
    var name: String { switch self { case .silk: "流光绢"; case .midnight: "午夜黑胶"; case .paper: "纸上唱片"; case .aurora: "极光"; case .mineradio: "Mineradio 舞台" } }
    var scheme: ColorScheme? { switch self { case .midnight, .aurora, .mineradio: .dark; case .paper: .light; case .silk: nil } }
}
enum ListeningLayout: String, CaseIterable, Identifiable {
    case record, album, lyrics, custom
    var id: String { rawValue }
    var name: String { switch self { case .record: "唱片与歌词"; case .album: "封面与歌词"; case .lyrics: "专注歌词"; case .custom: "自定义模块" } }
}
