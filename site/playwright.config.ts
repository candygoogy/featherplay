import { defineConfig, devices } from "@playwright/test";

// The browser tests run against the built site (`astro build` + `astro preview`), so they
// see exactly what ships: no dev-server scripts, no HMR.
const port = 4329;
export const wranglerPort = 8788;

/** Specs that measure motion in real time: the hero's moment and landing, the header's
 *  pills, the theme switch, the wheel's glide, the lyrics' glide and the sound. */
const timing = ["**/hero.spec.ts", "**/landing.spec.ts", "**/header.spec.ts", "**/theme.spec.ts", "**/smooth.spec.ts", "**/lyrics.spec.ts", "**/sound.spec.ts", "**/customise.spec.ts"];

// On CI (a smaller, shared Linux runner, .github/workflows/site.yml): twice the waits, one
// retry with a trace, and an HTML report the workflow uploads when a test fails.
const ci = !!process.env.CI;

export default defineConfig({
  testDir: "./tests",
  testIgnore: ["unit/**"],
  fullyParallel: true,
  forbidOnly: ci,
  retries: ci ? 1 : 0,
  reporter: ci ? [["list"], ["html", { open: "never", outputFolder: "./playwright-report" }]] : [["list"]],
  outputDir: "./test-results",
  expect: { timeout: ci ? 10_000 : 5_000 },
  use: {
    baseURL: `http://localhost:${port}`,
    colorScheme: "dark",
    actionTimeout: ci ? 10_000 : 5_000,
    trace: ci ? "on-first-retry" : "off",
  },
  projects: [
    { name: "chromium", testIgnore: ["unit/**", ...timing], use: { ...devices["Desktop Chrome"] } },
    // The specs that time animations frame by frame or against the clock run one at a time,
    // in their own worker, so a busy neighbour never stretches a frame they're measuring.
    // The rest stay parallel.
    { name: "timing", testMatch: timing, fullyParallel: false, workers: 1, use: { ...devices["Desktop Chrome"] } },
  ],
  webServer: [
    {
      command: `pnpm run build && pnpm exec astro preview --ignore-lock --port ${port}`,
      url: `http://localhost:${port}/`,
      reuseExistingServer: !process.env.CI,
      timeout: 120_000,
      env: { ASTRO_TELEMETRY_DISABLED: "1" },
    },
    // The same files on Cloudflare's runtime (local), for real status codes (tests/notfound.spec.ts).
    {
      command: `node scripts/wrangler-dev.mjs ${wranglerPort}`,
      url: `http://127.0.0.1:${wranglerPort}/`,
      reuseExistingServer: !process.env.CI,
      timeout: 120_000,
    },
  ],
});
