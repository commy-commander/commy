// swift-tools-version:5.5

import PackageDescription

let package = Package(
  name: "commy-ios-push",
  platforms: [
    .iOS(.v14)
  ],
  products: [
    .library(name: "commy-ios-push", type: .static, targets: ["CommyIosPush"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(name: "CommyIosPush", dependencies: [.byName(name: "Tauri")], path: "Sources/CommyIosPush")
  ]
)
