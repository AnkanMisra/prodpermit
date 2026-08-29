import { describe, expect, test } from "vitest";

import { WebMcpRegistry, type ToolActivity } from "@/lib/webmcp/registry";

class FakeModelContext {
  readonly registrations: Array<{
    tool: WebMCP.ModelContextTool;
    signal: AbortSignal | undefined;
  }> = [];

  async registerTool(
    tool: WebMCP.ModelContextTool,
    options?: WebMCP.ModelContextRegisterToolOptions
  ): Promise<void> {
    this.registrations.push({ tool, signal: options?.signal });
  }
}

describe("WebMcpRegistry", () => {
  test("registers a tool once and aborts its registration on disposal", async () => {
    const modelContext = new FakeModelContext();
    const registry = new WebMcpRegistry({ modelContext });
    const tool = {
      name: "inspect_incident",
      description: "Inspect the active incident.",
      inputSchema: { type: "object", properties: {} },
      annotations: { readOnlyHint: true },
      execute: () => ({ ok: true })
    } satisfies WebMCP.ModelContextTool;

    await registry.register(tool, "read-only");
    await registry.register(tool, "read-only");

    expect(modelContext.registrations).toHaveLength(1);
    expect(registry.tools()).toEqual([
      { name: "inspect_incident", classification: "read-only" }
    ]);

    registry.dispose();
    expect(modelContext.registrations[0]?.signal?.aborted).toBe(true);
  });

  test("reports successful tool execution activity", async () => {
    const modelContext = new FakeModelContext();
    const activity: ToolActivity[] = [];
    const registry = new WebMcpRegistry({
      modelContext,
      onActivity: (event) => activity.push(event)
    });
    await registry.register(
      {
        name: "inspect_incident",
        description: "Inspect the active incident.",
        execute: () => ({ ok: true })
      },
      "read-only"
    );
    const registered = modelContext.registrations[0]?.tool;
    if (!registered) {
      throw new Error("expected a registered tool");
    }

    await registered.execute({}, { signal: new AbortController().signal });

    expect(activity.map((event) => event.status)).toEqual(["running", "succeeded"]);
    expect(activity.every((event) => event.toolName === "inspect_incident")).toBe(true);
  });

  test("replaces a dynamic tool when its authority fingerprint changes", async () => {
    const modelContext = new FakeModelContext();
    const registry = new WebMcpRegistry({ modelContext });
    const tool = {
      name: "execute_approved_recovery",
      description: "Execute one approved recovery.",
      inputSchema: { type: "object", properties: { planId: { type: "string" } } },
      execute: () => ({ ok: true })
    } satisfies WebMCP.ModelContextTool;

    await registry.register(tool, "execution", "fingerprint-a");
    const firstSignal = modelContext.registrations[0]?.signal;
    await registry.register(tool, "execution", "fingerprint-b");

    expect(modelContext.registrations).toHaveLength(2);
    expect(firstSignal?.aborted).toBe(true);
    expect(modelContext.registrations[1]?.signal?.aborted).toBe(false);
    expect(registry.tools()).toEqual([
      { name: "execute_approved_recovery", classification: "execution" }
    ]);
  });
});
