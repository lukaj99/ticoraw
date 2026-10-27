// End-to-end tests of the built converter (web/dist) in headless Chromium.
//
//   sh build.sh && TICORAW_SAMPLES=<conformance cache> npx playwright test
//
// TICORAW_SAMPLES is a directory of <id>.nef files as conformance/fetch.sh writes them.
// Without it the conversion tests are skipped locally and fail in CI.
//
// Traces, screenshots and videos are off: a trace embeds the page's files, including the
// compiled decoder, which must not end up in an uploadable artifact (patent gate).

import { defineConfig, devices } from "@playwright/test";

const port = Number(process.env.PORT ?? 4173);

export default defineConfig({
  testDir: "test/e2e",
  timeout: 240_000,
  expect: { timeout: 15_000 },
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  reporter: "list",
  use: {
    baseURL: `http://127.0.0.1:${port}/`,
    acceptDownloads: true,
    trace: "off",
    screenshot: "off",
    video: "off",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: `node scripts/serve.mjs dist ${port}`,
    url: `http://127.0.0.1:${port}/index.html`,
    reuseExistingServer: !process.env.CI,
  },
});
