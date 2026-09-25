// Every `@objc` handler in `ios/Sources/FoundationModelsPlugin/FoundationModelsPlugin.swift`, and
// nothing else. This list is the generator input for `permissions/`: a handler missing here loses
// its permission and the invoke fails at runtime with "not allowed". 2 handlers, 2 permissions.
const COMMANDS: &[&str] = &["availability", "respond"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .try_build()
        .expect("failed to build Foundation Models plugin permissions");

    // Xcode 27 internalizes @_cdecl exports. The Swift target is `FoundationModelsPlugin`, not the
    // package name, so swift-rs does not promote the bridge symbol; promote it after SwiftPM runs.
    // Same repair as plugins/roomplan/build.rs.
    promote_ios_export("tauri-plugin-foundation-models", "_init_plugin_foundation_models");
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
