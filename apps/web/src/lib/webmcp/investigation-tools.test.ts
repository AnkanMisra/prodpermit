import { describe, expect, test } from "vitest";

import { createInvestigationTools } from "@/lib/webmcp/investigation-tools";

describe("investigation tools", () => {
  test("registers only the four bounded investigation capabilities", () => {
    const tools = createInvestigationTools({
      inspectIncident: async () => ({ ok: true }),
      compareReleases: async () => ({ ok: true }),
      queryLogs: async () => ({ ok: true }),
      runDiagnostic: async () => ({ ok: true })
    });

    expect(tools.map(({ tool }) => tool.name)).toEqual([
      "inspect_incident",
      "compare_releases",
      "query_logs",
      "run_diagnostic"
    ]);
    expect(tools.some(({ tool }) => tool.name === "execute_approved_recovery")).toBe(false);
    expect(tools.find(({ tool }) => tool.name === "query_logs")?.tool.annotations).toEqual({
      readOnlyHint: true,
      untrustedContentHint: true
    });
  });
});
