// swift-tools-version: 6.0
import PackageDescription

// No dependencies on purpose. FoundationModels and Network both ship in the
// SDK, and the whole binary is two routes on loopback -- a server framework
// would be more code to audit than the thing it serves.
let package = Package(
    name: "foundation-models",
    // The string form rather than `.v26`, which needs swift-tools-version 6.2
    // and would pin this manifest to a newer toolchain than the code needs.
    platforms: [.macOS("26.0")],
    targets: [
        .executableTarget(name: "afm-server", path: "Sources/afm-server")
    ]
)
