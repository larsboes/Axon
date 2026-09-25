// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "tauri-plugin-foundation-models",
  platforms: [
    // The app's floor stays iOS 16. every FoundationModels use is behind `#available(iOS 26, *)`,
    // which makes the linker weak-link the framework, so older systems load the plugin and report `osTooOld`.
    .iOS(.v16)
  ],
  products: [
    .library(
      name: "tauri-plugin-foundation-models",
      type: .static,
      targets: ["FoundationModelsPlugin"]
    )
  ],
  dependencies: [
    // Same generated Tauri API package as plugins/device-identity, so every plugin links one
    // Tauri runtime.
    .package(name: "Tauri", path: "../../roomplan/.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "FoundationModelsPlugin",
      dependencies: [.product(name: "Tauri", package: "Tauri")],
      path: "Sources/FoundationModelsPlugin"
    )
  ]
)
