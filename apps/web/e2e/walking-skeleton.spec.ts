import { expect, test } from "@playwright/test";
import { z } from "zod";

test.beforeEach(async ({ context }) => {
  await context.addInitScript(() => {
    const tools = new Map<string, WebMCP.ModelContextTool>();
    const modelContext = {
      async registerTool(
        tool: WebMCP.ModelContextTool,
        options?: WebMCP.ModelContextRegisterToolOptions
      ) {
        if (tools.has(tool.name)) {
          throw new DOMException("Tool already registered", "InvalidStateError");
        }
        tools.set(tool.name, tool);
        options?.signal?.addEventListener(
          "abort",
          () => {
            tools.delete(tool.name);
          },
          { once: true }
        );
      },
      names() {
        return [...tools.keys()].toSorted();
      },
      async invoke(name: string, input: Record<string, unknown>) {
        const tool = tools.get(name);
        if (!tool) {
          throw new Error(`Tool not registered: ${name}`);
        }
        return tool.execute(input, { signal: new AbortController().signal });
      }
    };
    Object.defineProperty(document, "modelContext", {
      configurable: true,
      value: modelContext
    });
  });
});

test("renders the Rust-backed broken incident without browser errors", async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      consoleErrors.push(message.text());
    }
  });

  const response = await page.goto("/");
  expect(response?.status()).toBe(200);
  expect(response?.headers()["origin-agent-cluster"]).toBe("?1");
  expect(response?.headers()["permissions-policy"]).toBe("tools=(self)");

  await expect(page.getByRole("heading", { name: "checkout-api" })).toBeVisible();
  await expect(page.getByRole("status")).toContainText("Critical");
  await expect(page.getByText("18.7%", { exact: true })).toBeVisible();
  await expect(page.getByText("1,420 ms")).toBeVisible();
  await expect(page.getByRole("img", { name: "Checkout error rate over time" })).toBeVisible();

  expect(consoleErrors).toEqual([]);
  await page.screenshot({
    path: "../../output/playwright/phase-2-walking-skeleton.png",
    animations: "disabled"
  });
});

test("investigates the incident through registered WebMCP tools", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "checkout-api" })).toBeVisible();
  await expect(page.getByText("WebMCP supported")).toBeVisible();

  const names = await page.evaluate(() => {
    const modelContext = Object.getOwnPropertyDescriptor(document, "modelContext")?.value;
    return modelContext.names();
  });
  expect(names).toEqual([
    "compare_releases",
    "inspect_incident",
    "prepare_recovery",
    "query_logs",
    "run_diagnostic",
    "verify_recovery"
  ]);
  expect(names).not.toContain("execute_approved_recovery");

  await invokeTool(page, "inspect_incident", {
    service: "checkout-api",
    windowMinutes: 30
  });
  await invokeTool(page, "compare_releases", {
    baselineRelease: "release_283",
    candidateRelease: "release_284"
  });
  await expect(page.getByText("database.auth_mode")).toBeVisible();
  await expect(page.getByText("Suspected regression", { exact: true })).toBeVisible();

  await invokeTool(page, "query_logs", { windowMinutes: 30, limit: 25 });
  await expect(page.getByText("DB_AUTH_METHOD_MISMATCH").first()).toBeVisible();
  await expect(page.getByText("External text")).toBeVisible();

  await invokeTool(page, "run_diagnostic", { kind: "database_connectivity" });
  await expect(page.getByRole("heading", { name: "Database connectivity" })).toBeVisible();
  await expect(page.getByText("Failed", { exact: true })).toBeVisible();
  await expect(page.getByText("run_diagnostic: succeeded")).toBeVisible();

  await page.setViewportSize({ width: 1280, height: 1600 });
  await page.screenshot({
    path: "../../output/playwright/phase-3-investigation.png",
    animations: "disabled"
  });
});

test("keeps the telemetry chart scaled inside its panel", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "checkout-api" })).toBeVisible();

  const metrics = await page.evaluate(() => {
    const html = document.documentElement;
    const chart = document.querySelector(".telemetry-chart");
    const panel = document.querySelector(".telemetry-panel");
    if (!(chart instanceof SVGElement) || !(panel instanceof HTMLElement)) {
      throw new Error("Telemetry chart or panel is missing.");
    }
    const chartBox = chart.getBoundingClientRect();
    const panelBox = panel.getBoundingClientRect();
    return {
      overflowX: html.scrollWidth - html.clientWidth,
      chartWidth: chartBox.width,
      chartHeight: chartBox.height,
      panelWidth: panelBox.width,
      panelHeight: panelBox.height,
      widthAttr: chart.getAttribute("width"),
      heightAttr: chart.getAttribute("height"),
      preserveAspectRatio: chart.getAttribute("preserveAspectRatio")
    };
  });

  expect(metrics.overflowX).toBe(0);
  expect(metrics.widthAttr).toBe("640");
  expect(metrics.heightAttr).toBe("220");
  expect(metrics.preserveAspectRatio).toBe("xMidYMid meet");
  expect(metrics.chartHeight).toBeLessThanOrEqual(metrics.panelHeight);
  expect(metrics.chartWidth).toBeLessThanOrEqual(metrics.panelWidth);
  expect(metrics.chartHeight / metrics.chartWidth).toBeCloseTo(220 / 640, 2);
});

test("gates one exact recovery behind human approval", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "checkout-api" })).toBeVisible();

  const diagnosticOutput = await invokeTool(page, "run_diagnostic", {
    kind: "database_connectivity"
  });
  const diagnosticId = z
    .object({ ok: z.literal(true), data: z.object({ id: z.string().uuid() }) })
    .parse(diagnosticOutput).data.id;
  const preparedOutput = await invokeTool(page, "prepare_recovery", {
    targetRelease: "release_283",
    reason: "Rollback the database authentication regression.",
    evidenceRefs: ["log_db_auth_1", diagnosticId]
  });
  const prepared = z
    .object({
      ok: z.literal(true),
      data: z.object({ planId: z.string().uuid(), fingerprint: z.string().length(64) })
    })
    .parse(preparedOutput).data;

  await expect(page.getByRole("heading", { name: "Recovery plan" })).toBeVisible();
  await expect(page.getByText(prepared.fingerprint)).toBeVisible();
  await expect(page.getByText("log_db_auth_1")).toBeVisible();
  await expect(page.getByText("Production changed").locator("..")).toContainText("No");
  expect(await registeredToolNames(page)).not.toContain("execute_approved_recovery");

  await page.getByRole("button", { name: "Approve exact plan" }).click();
  await expect(page.getByText("execute_approved_recovery")).toBeVisible();
  expect(await registeredToolNames(page)).toContain("execute_approved_recovery");

  await invokeTool(page, "execute_approved_recovery", { planId: prepared.planId });
  await expect(page.getByRole("status").first()).toContainText("Healthy");
  await expect(page.getByText("Production changed").locator("..")).toContainText("Yes");
  expect(await registeredToolNames(page)).not.toContain("execute_approved_recovery");

  await invokeTool(page, "verify_recovery", { planId: prepared.planId });
  await expect(page.getByText("Recovery verified")).toBeVisible();
  await expect(page.getByText("DB_CONNECTION_OK")).toBeVisible();

  await page.getByRole("button", { name: "Reset scenario" }).click();
  await expect(page.getByText("Critical", { exact: true })).toBeVisible();
  await expect(page.getByText("release_284").first()).toBeVisible();
  await expect(page.getByText(/Ask the agent to prepare the safest recovery/)).toBeVisible();
  expect(await registeredToolNames(page)).not.toContain("execute_approved_recovery");
});

test("restores and revokes an approved capability across reload", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "checkout-api" })).toBeVisible();
  const diagnosticOutput = await invokeTool(page, "run_diagnostic", {
    kind: "database_connectivity"
  });
  const diagnosticId = z
    .object({ ok: z.literal(true), data: z.object({ id: z.string().uuid() }) })
    .parse(diagnosticOutput).data.id;
  await invokeTool(page, "prepare_recovery", {
    targetRelease: "release_283",
    reason: "Rollback the database authentication regression.",
    evidenceRefs: ["log_db_auth_1", diagnosticId]
  });
  await page.getByRole("button", { name: "Approve exact plan" }).click();
  await expect(page.getByText("execute_approved_recovery")).toBeVisible();

  await page.reload();
  await expect(page.getByText("execute_approved_recovery")).toBeVisible();
  expect(await registeredToolNames(page)).toContain("execute_approved_recovery");

  await page.getByRole("button", { name: "Revoke approval" }).click();
  await expect(page.getByText("Execution capability absent", { exact: false })).toBeVisible();
  expect(await registeredToolNames(page)).not.toContain("execute_approved_recovery");
});

async function invokeTool(
  page: import("@playwright/test").Page,
  name: string,
  input: Record<string, unknown>
) {
  return page.evaluate(
    async ({ toolName, toolInput }) => {
      const modelContext = Object.getOwnPropertyDescriptor(document, "modelContext")?.value;
      return modelContext.invoke(toolName, toolInput);
    },
    { toolName: name, toolInput: input }
  );
}

async function registeredToolNames(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const modelContext = Object.getOwnPropertyDescriptor(document, "modelContext")?.value;
    return modelContext.names();
  });
}
