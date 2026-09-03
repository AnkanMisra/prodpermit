import { defineConfig, devices } from "@playwright/test";

const apiPort = 18_080;
const webPort = 13_000;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: "list",
  use: {
    baseURL: `http://localhost:${webPort}`,
    trace: "retain-on-failure"
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] }
    }
  ],
  webServer: [
    {
      command:
        `PORT=${apiPort} ALLOWED_ORIGIN=http://localhost:${webPort} ` +
        "DATABASE_URL=sqlite:///tmp/prodpermit-playwright.db?mode=rwc " +
        "bun run --cwd ../.. dev:api",
      url: `http://localhost:${apiPort}/api/health`,
      reuseExistingServer: true,
      timeout: 180_000
    },
    {
      command:
        `BACKEND_URL=http://127.0.0.1:${apiPort} ` +
        `bun run dev --port ${webPort}`,
      url: `http://localhost:${webPort}`,
      reuseExistingServer: true,
      timeout: 180_000
    }
  ]
});
