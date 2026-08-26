import { z } from "zod";

import type { ToolClassification } from "@/lib/webmcp/registry";

type Registration = {
  tool: WebMCP.ModelContextTool;
  classification: ToolClassification;
};

type RecoveryHandlers = {
  prepareRecovery: (
    input: { targetRelease: string; reason: string; evidenceRefs: string[] },
    signal: AbortSignal
  ) => Promise<unknown>;
  verifyRecovery: (input: { planId: string }, signal: AbortSignal) => Promise<unknown>;
};

const prepareSchema = z.object({
  targetRelease: z.string().regex(/^release_[0-9]+$/),
  reason: z.string().trim().min(1).max(240),
  evidenceRefs: z.array(z.string().min(1)).min(1).max(8)
});
const planSchema = z.object({ planId: z.string().uuid() });

export function createRecoveryTools(handlers: RecoveryHandlers): Registration[] {
  return [
    {
      classification: "staging",
      tool: {
        name: "prepare_recovery",
        title: "Prepare recovery",
        description:
          "Prepare an exact checkout-api rollback plan from verified evidence. This does not change production state and requires later human approval.",
        inputSchema: {
          type: "object",
          additionalProperties: false,
          properties: {
            targetRelease: { type: "string", pattern: "^release_[0-9]+$" },
            reason: { type: "string", minLength: 1, maxLength: 240 },
            evidenceRefs: {
              type: "array",
              minItems: 1,
              maxItems: 8,
              items: { type: "string", minLength: 1 }
            }
          },
          required: ["targetRelease", "reason", "evidenceRefs"]
        },
        annotations: { readOnlyHint: false },
        execute: (input, { signal }) => handlers.prepareRecovery(prepareSchema.parse(input), signal)
      }
    },
    {
      classification: "read-only",
      tool: {
        name: "verify_recovery",
        title: "Verify recovery",
        description:
          "Verify the outcome of an executed recovery plan, including current release, service health, and database diagnostic state.",
        inputSchema: planInputSchema(),
        annotations: { readOnlyHint: true },
        execute: (input, { signal }) => handlers.verifyRecovery(planSchema.parse(input), signal)
      }
    }
  ];
}

export function createExecutionTool(
  execute: (input: { planId: string }, signal: AbortSignal) => Promise<unknown>
): Registration {
  return {
    classification: "execution",
    tool: {
      name: "execute_approved_recovery",
      title: "Execute approved recovery",
      description:
        "Execute one exact approved recovery plan. The Rust backend independently verifies approval, expiry, session, active release, target, and replay state.",
      inputSchema: planInputSchema(),
      annotations: { readOnlyHint: false },
      execute: (input, { signal }) => execute(planSchema.parse(input), signal)
    }
  };
}

function planInputSchema() {
  return {
    type: "object",
    additionalProperties: false,
    properties: { planId: { type: "string", format: "uuid" } },
    required: ["planId"]
  };
}
