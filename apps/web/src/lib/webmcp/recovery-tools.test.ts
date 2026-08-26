import { describe, expect, test } from "vitest";

import { createExecutionTool, createRecoveryTools } from "@/lib/webmcp/recovery-tools";

describe("recovery tools", () => {
  test("keeps execution separate from the initial recovery tools", () => {
    const initial = createRecoveryTools({
      prepareRecovery: async () => ({ ok: true }),
      verifyRecovery: async () => ({ ok: true })
    });
    const execution = createExecutionTool(async () => ({ ok: true }));

    expect(initial.map(({ tool }) => tool.name)).toEqual([
      "prepare_recovery",
      "verify_recovery"
    ]);
    expect(initial.some(({ tool }) => tool.name === "execute_approved_recovery")).toBe(false);
    expect(execution.tool.name).toBe("execute_approved_recovery");
    expect(execution.classification).toBe("execution");
  });
});
