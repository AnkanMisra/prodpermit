import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: "list",
  use: {
    baseURL: "http://localhost:3000",
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
      command: "bun run --cwd ../.. dev:api",
      url: "http://localhost:8080/api/health",
      reuseExistingServer: true,
      timeout: 180_000
    },
    {
      command: "bun run --cwd ../.. dev:web",
      url: "http://localhost:3000",
      reuseExistingServer: true,
      timeout: 180_000
    }
  ]
});
