import { defineConfig, devices } from '@playwright/test'

const baseURL = process.env.PLAYWRIGHT_BASE_URL || 'http://127.0.0.1:41739'
const artifactDir = process.env.UI_SMOKE_ARTIFACT_DIR || '../artifacts/ui-smoke/local'

export default defineConfig({
  testDir: './tests',
  testMatch: /ui-smoke\.spec\.mjs/,
  timeout: 30_000,
  expect: {
    timeout: 5_000,
  },
  fullyParallel: false,
  workers: 1,
  reporter: [
    ['list'],
    ['html', { outputFolder: `${artifactDir}/playwright-report`, open: 'never' }],
  ],
  outputDir: `${artifactDir}/test-results`,
  use: {
    baseURL,
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
