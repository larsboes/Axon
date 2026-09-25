// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "tauri-plugin-device-identity",
  platforms: [
    .iOS(.v16)
  ],
  products: [
    .library(
      name: "tauri-plugin-device-identity",
      type: .static,
      targets: ["DeviceIdentityPlugin"]
    )
  ],
  dependencies: [
    // The Tauri API is generated for the existing iOS plugin setup. Reusing that generated
    // package keeps this small plugin on the same Tauri runtime as RoomPlan.
    .package(name: "Tauri", path: "../../roomplan/.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "DeviceIdentityPlugin",
      dependencies: [.product(name: "Tauri", package: "Tauri")],
      path: "Sources/DeviceIdentityPlugin"
    )
  ]
)
