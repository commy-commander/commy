// swift-tools-version:5.3

import PackageDescription

let package = Package(
  name: "commy-apple-sign-in",
  platforms: [
    .iOS(.v13)
  ],
  products: [
    .library(name: "commy-apple-sign-in", type: .static, targets: ["commy-apple-sign-in"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(name: "commy-apple-sign-in", dependencies: [.byName(name: "Tauri")], path: "Sources")
  ]
)
