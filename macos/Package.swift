// swift-tools-version: 6.2
import PackageDescription
import Foundation

let nativeStage = ProcessInfo.processInfo.environment["RMUSIC_NATIVE_V2"] == "1"
let visualAssets: Target.Dependency = .product(name: "RMusicVisualAssets", package: "VisualAssets")
var products: [Product] = [.executable(name: "RMusic", targets: ["RMusic"]), .executable(name: "RMusicBridge", targets: ["RMusicBridge"])]
var targets: [Target] = [
    .target(name: "RMusicKit", dependencies: [visualAssets] + (nativeStage ? [.target(name: "MineradioNative")] : []), swiftSettings: nativeStage ? [.define("RMUSIC_NATIVE_MINERADIO")] : []),
    .executableTarget(name: "RMusic", dependencies: ["RMusicKit"]),
    .executableTarget(name: "RMusicBridge", dependencies: ["RMusicKit"]),
    .testTarget(name: "RMusicKitTests", dependencies: ["RMusicKit"])
]
if nativeStage {
    products.append(.executable(name: "RMusicNative", targets: ["RMusicNative"]))
    targets.append(contentsOf: [
        .target(name: "MineradioNative", resources: [.copy("Resources")]),
        .executableTarget(name: "RMusicNative", dependencies: ["RMusicKit"]),
        .testTarget(name: "MineradioNativeTests", dependencies: ["MineradioNative"])
    ])
}
let package = Package(name: "RMusicMac", platforms: [.macOS("27.0")], products: products, dependencies: [.package(path: "VisualAssets")], targets: targets)
