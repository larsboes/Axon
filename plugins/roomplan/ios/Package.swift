// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "tauri-plugin-roomplan",
  platforms: [
    .iOS(.v16)
  ],
  products: [
    .library(
      name: "tauri-plugin-roomplan",
      type: .static,
      targets: ["RoomPlanPlugin"]
    )
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "RoomPlanPlugin",
      dependencies: [.product(name: "Tauri", package: "Tauri")],
      path: "Sources/RoomPlanPlugin"
    )
  ]
)
