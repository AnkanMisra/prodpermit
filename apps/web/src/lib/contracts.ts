import { z } from "zod";

const sessionIdSchema = z.string().uuid().brand<"SessionId">();
const releaseIdSchema = z
  .string()
  .regex(/^release_[0-9]+$/)
  .brand<"ReleaseId">();
const timestampSchema = z.string().datetime({ offset: true });

export const incidentSnapshotSchema = z.object({
  session: z.object({
    id: sessionIdSchema,
    createdAt: timestampSchema,
    expiresAt: timestampSchema,
    generation: z.number().int().positive()
  }),
  incident: z.object({
    id: z.string().min(1),
    serviceId: z.string().min(1),
    title: z.string().min(1),
    summary: z.string().min(1),
    status: z.enum(["active", "resolved"]),
    startedAt: timestampSchema
  }),
  health: z.object({
    status: z.enum(["healthy", "critical"]),
    errorRatePercent: z.number().nonnegative(),
    p95LatencyMs: z.number().int().nonnegative(),
    requestRateRps: z.number().int().nonnegative(),
    currentRelease: releaseIdSchema
  }),
  releases: z.array(
    z.object({
      id: releaseIdSchema,
      state: z.enum(["healthy_baseline", "deployed_faulty", "staged"]),
      commitSha: z.string().min(1),
      description: z.string().min(1),
      deployedAt: timestampSchema.nullable()
    })
  ),
  telemetry: z
    .array(
      z.object({
        timestamp: timestampSchema,
        errorRatePercent: z.number().nonnegative(),
        p95LatencyMs: z.number().int().nonnegative(),
        requestRateRps: z.number().int().nonnegative()
      })
    )
    .min(1)
});

export const incidentDataResponseSchema = z.object({
  data: incidentSnapshotSchema
});

export const apiErrorResponseSchema = z.object({
  error: z.object({
    code: z.string(),
    message: z.string(),
    requestId: z.string().uuid(),
    retryable: z.boolean()
  })
});

export const releaseComparisonSchema = z.object({
  baseline: incidentSnapshotSchema.shape.releases.element,
  candidate: incidentSnapshotSchema.shape.releases.element,
  configurationDiff: z.array(
    z.object({
      key: z.string(),
      baselineValue: z.string(),
      candidateValue: z.string(),
      suspectedRegression: z.boolean()
    })
  ),
  dependencyDiff: z.array(
    z.object({
      key: z.string(),
      baselineValue: z.string(),
      candidateValue: z.string(),
      suspectedRegression: z.boolean()
    })
  )
});

export const logEventSchema = z.object({
  id: z.string(),
  recordedAt: timestampSchema,
  severity: z.enum(["info", "warn", "error"]),
  code: z.string(),
  component: z.string(),
  message: z.string(),
  untrusted: z.boolean()
});

export const diagnosticResultSchema = z.object({
  id: z.string().uuid(),
  kind: z.literal("database_connectivity"),
  status: z.enum(["passed", "failed"]),
  code: z.string(),
  summary: z.string(),
  evidence: z.string(),
  checkedAt: timestampSchema
});

export const recoveryPlanSchema = z.object({
  planId: z.string().uuid().brand<"PlanId">(),
  sessionId: sessionIdSchema,
  incidentId: z.string(),
  serviceId: z.string(),
  currentRelease: releaseIdSchema,
  targetRelease: releaseIdSchema,
  expectedCurrentRelease: releaseIdSchema,
  scenarioGeneration: z.number().int().positive(),
  reason: z.string(),
  supportingEvidence: z.array(z.string()),
  riskLevel: z.literal("low"),
  preconditions: z.array(z.string()),
  fingerprint: z.string().length(64),
  createdAt: timestampSchema,
  expiresAt: timestampSchema,
  approvedAt: timestampSchema.nullable(),
  executedAt: timestampSchema.nullable(),
  status: z.enum([
    "prepared",
    "approved",
    "executing",
    "executed",
    "rejected",
    "expired",
    "invalidated"
  ])
});

export const currentRecoverySchema = z.object({
  plan: recoveryPlanSchema.nullable(),
  executionCapability: z.discriminatedUnion("kind", [
    z.object({
      kind: z.literal("available"),
      planId: recoveryPlanSchema.shape.planId,
      fingerprint: z.string().length(64),
      expiresAt: timestampSchema
    }),
    z.object({
      kind: z.literal("absent"),
      reason: z.enum(["no_plan", "not_approved", "terminal", "expired", "invalidated"])
    })
  ])
});

const recoveryTelemetryEvidenceSchema = z.object({
  planId: recoveryPlanSchema.shape.planId,
  serviceId: z.string().min(1),
  releaseId: releaseIdSchema,
  scenarioGeneration: z.number().int().positive(),
  recordedAt: timestampSchema,
  errorRatePercent: z.number().nonnegative(),
  p95LatencyMs: z.number().int().nonnegative(),
  requestRateRps: z.number().int().nonnegative()
});

const recoveryDiagnosticEvidenceSchema = z.object({
  planId: recoveryPlanSchema.shape.planId,
  id: z.string().min(1),
  serviceId: z.string().min(1),
  releaseId: releaseIdSchema,
  scenarioGeneration: z.number().int().positive(),
  kind: z.literal("database_connectivity"),
  status: z.enum(["passed", "failed"]),
  code: z.string(),
  summary: z.string(),
  evidence: z.string(),
  checkedAt: timestampSchema
});

export const recoveryVerificationSchema = z.object({
  planId: z.string().uuid().brand<"PlanId">(),
  outcome: z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("passed") }),
    z.object({ kind: z.literal("mismatch"), mismatches: z.array(z.string()) })
  ]),
  previousRelease: releaseIdSchema,
  currentRelease: releaseIdSchema,
  healthStatus: z.enum(["healthy", "critical"]),
  diagnosticStatus: z.enum(["passed", "failed"]),
  before: z.object({
    release: releaseIdSchema,
    evidence: z.array(z.unknown())
  }),
  after: z.object({
    release: releaseIdSchema,
    healthStatus: z.enum(["healthy", "critical"]),
    incidentStatus: z.enum(["active", "resolved"]),
    telemetry: recoveryTelemetryEvidenceSchema,
    diagnostic: recoveryDiagnosticEvidenceSchema
  }),
  verifiedAt: timestampSchema
});

export const auditEventSchema = z.object({
  id: z.string().uuid(),
  eventType: z.string(),
  subjectId: z.string().nullable(),
  outcome: z.string(),
  detail: z.string(),
  recordedAt: timestampSchema
});

export type ApiError = z.infer<typeof apiErrorResponseSchema>["error"];
export type AuditEvent = z.infer<typeof auditEventSchema>;
export type DiagnosticResult = z.infer<typeof diagnosticResultSchema>;
export type IncidentSnapshot = z.infer<typeof incidentSnapshotSchema>;
export type LogEvent = z.infer<typeof logEventSchema>;
export type ReleaseComparison = z.infer<typeof releaseComparisonSchema>;
export type RecoveryPlan = z.infer<typeof recoveryPlanSchema>;
export type CurrentRecovery = z.infer<typeof currentRecoverySchema>;
export type RecoveryVerification = z.infer<typeof recoveryVerificationSchema>;
