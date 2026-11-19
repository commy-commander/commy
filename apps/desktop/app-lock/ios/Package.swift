// swift-tools-version:5.3

import PackageDescription

let package = Package(
  name: "commy-app-lock",
  platforms: [
    .iOS(.v13)
  ],
  products: [
    .library(name: "commy-app-lock", type: .static, targets: ["commy-app-lock"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(name: "commy-app-lock", dependencies: [.byName(name: "Tauri")], path: "Sources")
  ]
)
