// Every `@objc` handler in `ios/Sources/RoomPlanPlugin/RoomPlanPlugin.swift`, and nothing else.
// This list is the generator input for `permissions/`, so a command missing here loses its
// permission on the next build and the invoke fails at runtime with "not allowed" — a silent
// break of a working feature. Count both sides before editing: 11 handlers, 11 permissions.
const COMMANDS: &[&str] = &[
  "isAvailable",
  "previewReference",
  "currentCapture",
  "acceptPending",
  "rejectPending",
  "importLegacyReference",
  "checkPermissions",
  "requestPermissions",
  "startCapture",
  "stopCapture",
  "cancelCapture",
];

fn main() {
  tauri_plugin::Builder::new(COMMANDS)
    .ios_path("ios")
    .try_build()
    .expect("failed to build RoomPlan plugin permissions");
}
