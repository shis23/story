import { defineConfig, devices } from '@playwright/test'

// Dedicated config for CSP-behavior tests that drive real Chromium CSP
// enforcement (NOT jsdom/string checks). Kept separate from playwright.config.mjs
// (the app ui-smoke) so the two suites can run independently and the CSP tests
// do not require a running Vite server.
const artifactDir = process.env.CSP_ARTIFACT_DIR || '../artifacts/csp-repro'

export default defineConfig({
  testDir: './tests',
  testMatch: /csp-inheritance\.spec\.mjs/,
  timeout: 30_000,
  expect: {
    timeout: 8_000,
  },
  fullyParallel: false,
  workers: 1,
  reporter: [['list']],
  outputDir: `${artifactDir}/test-results`,
  use: {
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
})
