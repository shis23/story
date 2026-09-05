# Android Top-Bar Acceptance

Date: 2026-09-05 (Asia/Shanghai).

## Scope And Isolation

- Branch: `codex/android-topbar-unify`, based on `3b114fe`.
- Worktree: `C:/Users/Predator/ZCodeProject/storyforge/.worktrees/android-topbar-unify`.
- The main checkout and the concurrent review agent's edits were not modified.
- No merge or push was performed. The main checkout also has changes in
  `frontend/src/design/campaign/CampaignScreen.vue`; reconcile them when merging.
- Only the two user-identified screenshots were retrieved from the phone gallery.

## Findings And Fix

1. The main title was absolutely centered over the combined status-bar inset and
   toolbar, while its controls were positioned below the inset. It now shares
   the controls' grid row and truncates inside its available width.
2. Full-screen and side panels did not consistently apply system safe areas.
   Viewport-owning shells now apply `--sf-safe-*` once; their headers do not add
   another inset.
3. The mobile sidebar previously began below the status bar and exposed the main
   title behind it. Its background now covers the viewport from the top, while
   its contents respect the safe area.
4. Main, sidebar, inspector, panel, legacy overlay, campaign and Meta headers now
   share the 52 CSS-pixel `sf-toolbar` height.
5. Campaign file actions use an accessible overflow menu on phones and labelled
   icon buttons on wider screens. The detail title has its own row on phones;
   action buttons no longer squeeze it into an unnecessarily tall narrow column.
6. Removed the redundant Meta input-area bottom inset. Existing desktop colors,
   navigation and business logic were preserved.

## Verification

| Check | Result |
| --- | --- |
| Initial regression against the old layout | Failed as expected: header at y=0 instead of y=40 |
| `npm run test:mobile-chrome` | 5/5 passed |
| `npm run test:ui -- --maxWorkers=4` | 94/94 passed |
| `npm test` | 487/487 passed |
| Frontend production build | Passed |
| Android arm64 debug APK build | Passed |
| Non-destructive USB replacement installation | Passed |
| Cold restart, background/foreground | Passed |

Browser regression coverage uses real Vue components in an isolated fixture at
320, 411, 768 and 1280 CSS-pixel widths with simulated safe insets. It checks
header height/position, title/button collisions, sidebar coverage, campaign
title wrapping, panel close controls, menu action events and Escape focus
restoration. These fixture tests do not replace native backend acceptance.

Native checks used a Redmi 23117RK66C, Android 16 / API 36, WebView
143.0.7499.192, 1080x2400 physical pixels, DPR 2.625. Observed safe insets were
40 CSS pixels above and 16 below. Main, sidebar, campaign, connection, Meta and
inspector headers were at y=40 with height=52. The legacy card-library header
was at y=40.76 due to its existing outer border, with height=52.

The real soft keyboard reduced the visual viewport from 914.29 to 563.43 CSS
pixels without moving the connection header into the status bar. Dismissing it
restored the viewport. Native screenshots were inspected, not only DOM metrics.

Two campaigns, three synthetic cards, the original one-message conversation,
and its saved weather variable remained available. No uninstall, application
data reset, database edit, API-key entry, or paid model request was performed.
Cold-restart app logs contained no panic, startup checksum error, fatal signal,
or uncaught JavaScript error.

Existing build warnings remain: mixed static/dynamic imports, the main bundle
size warning, the identifier ending in `.app`, and Gradle deprecations. No
coverage percentage or broad backend regression claim is made for this UI fix.

## Local Evidence

Evidence directory:
`C:/Users/Predator/ZCodeProject/storyforge/artifacts/android-topbar-2026-09-05`.

- `screenshots/01-main.png`
- `screenshots/02-sidebar.png`
- `screenshots/03-campaign.png`
- `screenshots/04-campaign-menu.png`
- `screenshots/05-connection.png`
- `screenshots/06-connection-keyboard.png`
- `screenshots/07-meta.png`
- `screenshots/08-character-library.png`
- `screenshots/09-campaign-variables.png`
- `screenshots/10-inspector.png`
- `screenshots/11-cold-restart.png`
- `native-metrics.json`
- `cold-restart.log`
- `storyforge-topbar-arm64-debug.apk`

Installed APK SHA-256:
`697b642e0d1d92364669fc784655292f4c986f170e682db166c81bfcf90bd727`.

The native WebView loaded `assets/index-BCDTxlR3.js` from the repaired build.
Evidence and APKs are local ignored artifacts, not committed source files.

## Rebuild Notes

This worktree's generated Android project is independent. The Cargo target
directory was shared with the main checkout only for compiled dependency caches.
Avoid concurrent Android builds against that target.

Two environment issues were resolved during acceptance:

1. The shared cache did not regenerate `TauriActivity.kt` for the new worktree.
   Its generated copy was restored only after matching it byte-for-byte against
   the locked Tauri 2.11.5 template with the package/library substitutions.
   No tracked native implementation was changed.
2. Windows `core.autocrlf` changed the raw bytes embedded by `include_str!` in
   migrations V005-V008. The original installed database uses LF for these
   migrations and CRLF for V001-V004. The first replacement APKs therefore
   failed startup checksum validation for V005. Application data was preserved.
   After verifying that the SQL differed only in line endings, the original
   bytes were restored in this worktree and file timestamps refreshed to force
   Cargo to recompile `storyforge-infra-sqlite`. The final library was checked
   for the LF payload before installation, and the final APK booted successfully.

Do not bypass migration checksum validation or reset the phone database to
reproduce this build. Durable cross-platform migration-byte compatibility is a
separate backend concern; this branch intentionally does not change migration
SQL, checksum rules, or repository-wide line-ending policy.

Build environment:

```powershell
$env:ANDROID_HOME='C:/Users/Predator/android-sdk'
$env:NDK_HOME='C:/Users/Predator/android-sdk/ndk/27.2.12479018'
$env:JAVA_HOME='C:/Program Files/Eclipse Adoptium/jdk-17.0.19.10-hotspot'
$env:CARGO_TARGET_DIR='C:/Users/Predator/ZCodeProject/storyforge/target'
$env:CARGO_BUILD_JOBS='8'
# Run in this worktree's crates/tauri-app directory.
cargo tauri android build --debug --target aarch64 --ci --split-per-abi --apk
```

## Recording Follow-Up

The user's 17:11 recording exposed two gaps in the first acceptance:

- Full-screen panels inherited the centered dialog's scale/fade transition.
  During opening, the old top-right controls could show through the new header.
- AppV2 unconditionally opened mobile navigation when any management panel
  closed, even when the panel had been opened from the top-right toolbar.

Full-screen panels now appear opaque at their final position from the first
frame. Centered dialogs and side-drawer animations are unchanged. Closing the
seven management panels only closes the requested panel; plugin-list refresh
still runs. It no longer changes the sidebar state as an unrelated side effect.

Both issues were demonstrated with failing tests before the correction:
seven production-shell close tests reopened the sidebar, and the frame-sampling
test observed opacity=0. After correction, 101 UI tests, 487 logic tests, and
six browser layout/transition tests passed. The frontend and arm64 APK builds
also passed. The replacement APK was installed without clearing app data.

Native verification repeated the variable/summary open-close sequence with
real Android touch input. All 102 sampled panel-header frames had opacity=1,
y=40 and height=52. Four initial cycles and four recorded cycles did not open
the sidebar on close. The overflow menu was also opened and closed by touch.
The new recording was decoded and inspected, including its transition frames.

Additional local evidence in the same evidence directory:

- `user-recording-171100.mp4`: the user's six-second report.
- `recording-top.png`: extracted original transition frames.
- `followup-native-frames.json`: per-frame native measurements.
- `topbar-switching-fixed-verified.mp4`: synchronized native interaction capture.
- `recording-fixed-transitions.png`: inspected corrected transition frames.
- `screenshots/12-followup-menu.png`: native overflow-menu state.
- `storyforge-topbar-switching-arm64-debug.apk`: follow-up acceptance build.

Follow-up APK SHA-256:
`670a110ce983c43d1086cdda0b2a1019e53a852ca1b2d2c3b78f8ed9e88389d6`.

The follow-up WebView loaded `assets/index-B0_l8Xi5.js`. Its process-specific
startup/runtime log check found no panic, checksum mismatch, fatal signal or
uncaught JavaScript error. Temporary USB screen-on and debugging-forward
settings were restored after verification.
