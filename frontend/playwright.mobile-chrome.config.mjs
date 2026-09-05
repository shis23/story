import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  testMatch: /(?:mobile-chrome|workbench-motion|theme-palettes)\.spec\.mjs/,
  fullyParallel: false,
  workers: 1,
  timeout: 30_000,
  outputDir: '../artifacts/mobile-chrome/test-results',
  reporter: [['list']],
  use: {
    baseURL: 'http://127.0.0.1:41748',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'npm run dev -- --host 127.0.0.1 --port 41748',
    url: 'http://127.0.0.1:41748',
    reuseExistingServer: !process.env.CI,
  },
})
