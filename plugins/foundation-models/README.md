# Tauri Foundation Models plugin

The on-device rung of the app's model ladder (`dashboard/src/lib/intelligence/`).

- `availability` reports two models: the on-device `SystemLanguageModel` and, from iOS 27, Apple's
  `PrivateCloudComputeLanguageModel`. Each is `available` or `unavailable` with a reason
  (`osTooOld`, `deviceNotEligible`, `appleIntelligenceNotEnabled`, `modelNotReady`,
  `systemNotReady`, `unsupportedLocale`).
- `respond` runs one prompt on the on-device model only. Private Cloud Compute is reported, never
  called: sending a prompt there is a data-class decision this plugin does not make.
- A prompt that does not fit the context window is refused with `context_length_exceeded`, never
  truncated. The router then tries the next rung.

The app's floor stays iOS 16. Every framework use sits behind `#available`, so the linker
weak-links `FoundationModels` and an older phone loads the plugin and reports `osTooOld`.

Apple documents the requirement: [Foundation Models](https://developer.apple.com/documentation/foundationmodels)
needs iOS 26 and an [Apple Intelligence-capable device](https://support.apple.com/en-us/121115).
API shapes in the Swift source follow the installed SDK's `.swiftinterface`.

Desktop and Android builds expose nothing. On the Mac, `capabilities/foundation-models` serves the
same model over loopback.
