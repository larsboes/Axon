// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "tauri-plugin-local-network",
  platforms: [
    .iOS(.v16)
  ],
  products: [
    .library(
      name: "tauri-plugin-local-network",
      type: .static,
      targets: ["LocalNetworkPlugin"]
    )
  ],
  dependencies: [
    // Same generated Tauri API package as plugins/device-identity, so every plugin links one
    // Tauri runtime.
    .package(name: "Tauri", path: "../../roomplan/.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "LocalNetworkPlugin",
      dependencies: [.product(name: "Tauri", package: "Tauri")],
      path: "Sources/LocalNetworkPlugin"
    )
  ]
)
