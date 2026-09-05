# Frontend Polish And Android Integration

Date: 2026-09-05 (Asia/Shanghai).

## Integration

Integrated the source changes from `codex/android-topbar-unify`:

- `b0467a4`: shared Android safe areas and toolbar heights; responsive file actions.
- `26151cb`: opaque, stationary full-screen panels; closing management panels
  no longer opens mobile navigation unexpectedly.

The main checkout already contained the architecture-review repairs. Its changes
were preserved. Android changes were applied to the working tree without a
commit, history merge, stash, reset or push. The overlapping CampaignScreen
changes were reconciled, retaining title wrapping and scrollable tabs. The
title/actions layout now also stacks at tablet widths below 1024px.

This integrates the Android UI fixes, not every finding from its initial device
acceptance. The existing Android credential-saving finding remains separate.
No phone package was rebuilt or installed during this frontend pass.

## Presentation

- TopBar uses in-flow grid tracks, labelled Lucide icon controls with 44px hit
  targets, and full-title tooltips. Removed the floating summary/variables box.
- Neutral paper and graphite surfaces, teal actions and distinct status colors.
  Updated light and night themes together; retained the existing component system.
- Composer uses a compact segmented mode selector and one input surface.
- Content enters over 240ms; navigation and inspector drawers slide; process
  disclosures expand; command buttons have brief press feedback.
- Only the topbar activity line animates continuously while writing. Story text
  is not animated per token. Stopping writing removes the activity line.
- Reduced-motion preference removes ongoing animation, animation delays and
  press scaling while retaining visible state feedback.
- Viewport roots and full-screen panel headers never scale or fade. Sidebar
  runtime slots remain mounted once across open/close and responsive changes.

## Verification

| Check | Result |
| --- | --- |
| Frontend Node tests | 491 passed |
| Vue component tests | 110 passed |
| Layout and motion browser tests | 16 passed |
| CSP browser tests | 9 passed |
| Production application-shell smoke tests | 2 passed |
| Frontend production build | Passed |
| Windows native debug build | Passed |
| Git diff whitespace check | Passed |

Browser coverage includes widths 320, 411, 480, 720, 768, 1024 and 1280, simulated
safe areas, long unbroken titles, icon target sizes, menu actions and keyboard
focus, frame-sampled drawer/disclosure motion, reduced motion, unsent-input
preservation, and light/night screenshots. Measurements of moving parents and
children are collected in the same animation frame.

Windows verification used the production frontend and real Tauri IPC with only
the previous review's synthetic SQLite data. The original native viewport was
480x900 CSS pixels at DPR 1.5, matching the screenshot's narrow-window scale.
A second viewport check used 1280x900 via WebView emulation.

- Topbar title and actions did not overlap; toolbar height 52px, controls 44px.
- All 21 sampled full-screen panel-header frames stayed opaque and stationary.
- Four summary/variable open-close cycles left mobile navigation closed.
- Unsent composer input survived opening and closing a management panel.
- No configured model connections, real API keys or paid requests were used.

Local evidence: `artifacts/frontend-polish-2026-09-05/`, including
`native-verification.json`, `native-panel-frames.json`, native screenshots and
`ui-smoke/`. Browser layout/motion screenshots are in
`artifacts/mobile-chrome/test-results/`.

Native tested executable SHA-256:
`DC688DD0DFD3750507E3B49491D32B73044E88403CE39BD94FD8A3812AA2A3C8`.

Production frontend entry: `assets/index-BOKLUyCh.js`.

No backend implementation changed in this pass. The earlier full Rust
workspace results are recorded separately in the architecture-review report;
they are not represented here as a fresh test run.

Existing Vite mixed-import and large-chunk warnings remain.

## Sidebar Collapse Follow-up

The desktop sidebar was previously forced visible and its close control was
hidden. Added an explicit desktop collapse state, independent of the mobile
drawer. The sidebar header can collapse it, and the topbar can collapse or
restore it. Navigation actions still only dismiss the mobile drawer. Crossing
the 1024px breakpoint clears stale mobile-open state without resetting the
desktop preference. Hidden sidebar content stays mounted once and is inert.

Fresh follow-up verification: 491 Node tests, 115 Vue tests, 17 layout/motion
browser tests, 9 CSP tests and 2 application-shell smoke tests passed (634 total).
Frontend production build and Windows native debug build both passed.

Native verification resized the actual Windows window using its title-bar
maximize/restore controls, with no WebView viewport emulation. The 480x900
viewport corresponded to a 720x1350 physical client area at DPR 1.5; maximized
width was about 1707 CSS pixels with a 2560-pixel physical client width.
Desktop collapse/restore, mobile close button/backdrop, stale-drawer clearing,
retained desktop preference, unsent draft and persistent sidebar DOM all passed.
Native pointer clicks also verified the desktop collapse and restore controls.
No frontend page errors occurred.

Evidence: `artifacts/sidebar-collapse-2026-09-05/native-verification.json` and
adjacent native screenshots. This follow-up supersedes the previous native
emulated desktop check for sidebar behavior.

Updated native executable SHA-256:
`E6368CE5F51D956437C8E9D5C8372EC7D2C4FB3BD5BA5FEC5CD1513509647AF3`.
Updated frontend entry: `assets/index-B8a6BFH2.js`.
No backend implementation, application permissions or phone installation changed.
