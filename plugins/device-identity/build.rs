// Keep this list aligned with the @objc handlers in the iOS plugin. The generated permission
// must expose exactly the native operation the Rust bridge calls.
const COMMANDS: &[&str] = &["getIdentity"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .try_build()
        .expect("failed to build device identity plugin permissions");

    // Xcode 27 internalizes @_cdecl exports. swift-rs can promote exports
    // whose Swift target name matches the package name, but this package's
    // Swift target is `DeviceIdentityPlugin`, so promote our bridge symbol
    // explicitly after SwiftPM produces the archive.
    promote_ios_export("tauri-plugin-device-identity", "_init_plugin_device_identity");
    // SwiftRs.o is intentionally excluded by swift-rs's package-name filter,
    // but these runtime helpers are required by the Rust swift-rs crate.
    for symbol in ["_release_object", "_retain_object", "_string_from_bytes"] {
        promote_ios_export("tauri-plugin-device-identity", symbol);
    }
}

fn promote_ios_export(package: &str, symbol: &str) {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("ios") {
        return;
    }

    let out_dir = std::env::var_os("OUT_DIR").expect("OUT_DIR is required");
    let archive_name = format!("lib{package}.a");
    let archive = find_file(std::path::Path::new(&out_dir), &archive_name)
        .unwrap_or_else(|| panic!("SwiftPM did not produce {archive_name}"));
    let sysroot = std::process::Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("failed to locate rustc sysroot");
    let sysroot = String::from_utf8(sysroot.stdout)
        .expect("rustc returned a non-UTF-8 sysroot")
        .trim()
        .to_owned();
    let objcopy = std::path::Path::new(&sysroot)
        .join("lib/rustlib")
        .join(format!("{}-apple-darwin", std::env::consts::ARCH))
        .join("bin/llvm-objcopy");
    if !objcopy.exists() {
        panic!("llvm-objcopy is required for Xcode 27 iOS builds: {}", objcopy.display());
    }
    let status = std::process::Command::new(objcopy)
        .arg(format!("--globalize-symbol={symbol}"))
        .arg(&archive)
        .status()
        .expect("failed to run llvm-objcopy");
    assert!(status.success(), "llvm-objcopy failed for {}", archive.display());
}

fn find_file(root: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|value| value.to_str()) == Some(name) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        }
    }
    None
}
