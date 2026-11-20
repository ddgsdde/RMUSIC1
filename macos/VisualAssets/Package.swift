// swift-tools-version: 6.2
import PackageDescription
let package = Package(
    name: "RMusicVisualAssets",
    platforms: [.macOS("27.0")],
    products: [.library(name: "RMusicVisualAssets", targets: ["RMusicVisualAssets"])],
    targets: [.target(name: "RMusicVisualAssets", resources: [.copy("Mineradio")])]
)
