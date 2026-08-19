import {defineConfig} from "@playwright/test"
import process from "node:process"

const APP_PORT: number = 5199
const PROVIDER_PORT: number = 5198
const PROVIDER_ORIGIN: string = `https://127.0.0.1:${PROVIDER_PORT}`
const LOCAL_BROWSER_CHANNEL: "chrome" | undefined =
  process.platform === "darwin" ? "chrome" : undefined

export default defineConfig({
  expect: {
    timeout: 10_000,
    toHaveScreenshot: {
      animations: "disabled",
      caret: "hide",
      maxDiffPixelRatio: 0.001,
    },
  },
  fullyParallel: true,
  outputDir: "test-results",
  reporter: process.env.CI ? [["line"], ["html", {open: "never"}]] : "list",
  snapshotPathTemplate: `{testDir}/snapshots/${process.platform}/{projectName}/{testFilePath}/{arg}{ext}`,
  testDir: "./e2e/scenarios",
  timeout: 60_000,
  use: {
    baseURL: `http://127.0.0.1:${APP_PORT}`,
    colorScheme: "dark",
    ignoreHTTPSErrors: true,
    locale: "en-US",
    screenshot: "only-on-failure",
    timezoneId: "UTC",
    trace: "retain-on-failure",
    video: "retain-on-failure",
    viewport: {height: 844, width: 390},
  },
  webServer: [
    {
      command: `bun e2e/support/provider-server.ts --port ${PROVIDER_PORT}`,
      ignoreHTTPSErrors: true,
      reuseExistingServer: !process.env.CI,
      url: `${PROVIDER_ORIGIN}/health`,
    },
    {
      command: `bun run dev --host 127.0.0.1 --port ${APP_PORT}`,
      env: {
        VITE_TONAPI_RATE_URL: `${PROVIDER_ORIGIN}/v2/rates?tokens=ton&currencies=usd`,
        VITE_TONCENTER_BASE_URL: PROVIDER_ORIGIN,
      },
      reuseExistingServer: !process.env.CI,
      url: `http://127.0.0.1:${APP_PORT}`,
    },
  ],
  workers: 1,
  projects: [
    {
      metadata: {compareScreenshots: true},
      name: "chromium",
      use: {browserName: "chromium", channel: LOCAL_BROWSER_CHANNEL},
    },
    {
      metadata: {compareScreenshots: false},
      name: "chromium-functional",
      use: {browserName: "chromium", channel: LOCAL_BROWSER_CHANNEL},
    },
  ],
})
