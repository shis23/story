# Theme Palette Selection

Date: 2026-09-05 (Asia/Shanghai).

## Behavior

- Sidebar footer offers teal, blue-gray and rose palettes through named native
  radio controls with keyboard navigation and visible selection/focus rings.
- Night reading is an independent accessible switch. Every palette supports
  both light and dark mode; palette changes do not dismiss navigation.
- Preferences use `storyforge-theme` and `storyforge-palette` in local storage.
  Existing dark-mode preferences remain compatible. Invalid values fall back
  to light/teal; unavailable storage does not break live selection or startup.
- Root CSS tokens cover backgrounds, text, borders, accents and selection.
  Semantic status colors retain their meanings. Browser chrome derives its
  theme color from the active background token.
- The first-paint bootstrap and runtime accept the same palette values.
  Switching appearance does not remount the sidebar or clear unsent drafts.
- No backend, permission, package dependency or phone installation changes.

## Verification

| Check | Result |
| --- | --- |
| Node tests | 496 passed |
| Vue component tests | 118 passed |
| Layout, motion and palette browser tests | 25 passed |
| CSP browser tests | 9 passed |
| Application-shell browser smoke tests | 2 passed |
| Frontend production build | Passed |
| Windows native debug build | Passed |

Total: 650 automated tests. Palette coverage includes all six combinations,
actual rendered button/navigation colors, text and filled-button contrast,
keyboard selection, refresh persistence, shared panel tokens, draft retention
and a 320x480 mobile window. Screenshots wait for finite transitions to finish.
Body text, secondary text and white-on-accent buttons meet the tested 4.5:1
contrast threshold; this is not a claim of a full accessibility audit.

The native build was checked at an actual 480x900 CSS viewport (720x1350 physical
pixels, DPR 1.5), without viewport emulation, using only synthetic review data.
All six palettes matched their stored preferences and browser chrome. The
footer fit the window; reload restored rose/night; panel navigation preserved
an unsent draft. No frontend page errors occurred.

Native evidence: `artifacts/theme-palettes-2026-09-05/native-verification.json`
and adjacent screenshots. Browser screenshots are copied into the same folder.

Native executable SHA-256:
`AD37A3041D3226D4960052CCFD59A5EA6FCF06B58DCFBBCEA16BA68561E348E1`.
Frontend entry: `assets/index-eaXuHEGP.js`.

Existing Vite mixed-import and large-chunk warnings remain. Changes are not
committed or pushed.

## Classic Palette Follow-up

Restored the original palette from `3b114fe:frontend/src/style.css` as the fourth
option, `classic` / "经典", without changing the default or current selection.
Both modes retain the original background, surface, text, border and accent
tokens. Existing layout fixes and motion remain in place.

| Mode | Background | Surface | Accent |
| --- | --- | --- | --- |
| Light | `#f6f3ec` | `#fffdf7` | `#9a6425` |
| Night | `#1b1712` | `#241f18` | `#d2a55e` |

Night-mode pale-gold filled controls use dark `#29241c` foregrounds for readable
contrast, including legacy accent/white utility pairs and the night switch.
The four swatches share one row with at least 44px hit targets.

Fresh verification: 497 Node tests, 119 Vue tests, 28 layout/motion/palette browser
tests, 9 CSP tests and 2 application-shell smoke tests passed (655 total).
Both builds passed. Native 480x900 verification confirmed original light/night
colors, readable button foregrounds, four accessible targets, reload persistence
and no page errors, with no viewport emulation or formal-data changes.

Evidence: `artifacts/classic-palette-2026-09-05/`.
Updated frontend entry: `assets/index-CL4cW1_f.js`.
Updated native executable SHA-256:
`87175EC421FAB7AAAB40D23B4D342622CAF80365D8CCD80A958AA3589E1AA391`.
