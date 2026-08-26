import {
  apiErrorResponseSchema,
  auditEventSchema,
  diagnosticResultSchema,
  incidentDataResponseSchema,
  incidentSnapshotSchema,
  logEventSchema,
  releaseComparisonSchema,
  recoveryPlanSchema,
  recoveryVerificationSchema,
  type DiagnosticResult,
  type AuditEvent,
  type IncidentSnapshot,
  type LogEvent,
  type RecoveryPlan,
  type RecoveryVerification,
  type ReleaseComparison
} from "@/lib/contracts";
import { z } from "zod";

export class ApiClientError extends Error {
  readonly code: string;
  readonly requestId: string;
  readonly retryable: boolean;

  constructor(input: {
    code: string;
    message: string;
    requestId: string;
    retryable: boolean;
  }) {
    super(input.message);
    this.name = "ApiClientError";
    this.code = input.code;
    this.requestId = input.requestId;
    this.retryable = input.retryable;
  }
}

export async function createOrResumeSession(
  signal?: AbortSignal
): Promise<IncidentSnapshot> {
  const response = await fetch("/api/backend/demo/sessions", {
    method: "POST",
    credentials: "same-origin",
    headers: { "X-Demo-Request": "1" },
    signal
  });
  const payload: unknown = await response.json();
  if (!response.ok) {
    const parsedError = apiErrorResponseSchema.safeParse(payload);
    if (parsedError.success) {
      throw new ApiClientError(parsedError.data.error);
    }
    throw new Error("The demo session could not be created.");
  }
  return incidentDataResponseSchema.parse(payload).data;
}

export async function fetchCurrentIncident(signal?: AbortSignal): Promise<IncidentSnapshot> {
  return requestData("/api/backend/incidents/current", incidentSnapshotSchema, { signal });
}

export async function compareReleases(
  input: { baselineRelease: string; candidateRelease: string },
  signal?: AbortSignal
): Promise<ReleaseComparison> {
  const query = new URLSearchParams(input);
  return requestData(
    `/api/backend/releases/compare?${query.toString()}`,
    releaseComparisonSchema,
    { signal }
  );
}

export async function queryLogs(
  input: { severity?: "info" | "warn" | "error"; windowMinutes: number; limit: number },
  signal?: AbortSignal
): Promise<LogEvent[]> {
  const query = new URLSearchParams({
    windowMinutes: String(input.windowMinutes),
    limit: String(input.limit)
  });
  if (input.severity) {
    query.set("severity", input.severity);
  }
  return requestData(`/api/backend/logs?${query.toString()}`, z.array(logEventSchema), { signal });
}

export async function runDiagnostic(signal?: AbortSignal): Promise<DiagnosticResult> {
  return requestData("/api/backend/diagnostics", diagnosticResultSchema, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Demo-Request": "1"
    },
    body: JSON.stringify({ kind: "database_connectivity" }),
    signal
  });
}

async function requestData<T>(
  path: string,
  schema: z.ZodType<T>,
  init: RequestInit
): Promise<T> {
  const response = await fetch(path, { ...init, credentials: "same-origin" });
  const payload: unknown = await response.json();
  if (!response.ok) {
    const parsedError = apiErrorResponseSchema.safeParse(payload);
    if (parsedError.success) {
      throw new ApiClientError(parsedError.data.error);
    }
    throw new Error("The request could not be completed.");
  }
  return z.object({ data: schema }).parse(payload).data;
}

export async function prepareRecovery(
  input: { targetRelease: string; reason: string; evidenceRefs: string[] },
  signal?: AbortSignal
): Promise<RecoveryPlan> {
  return requestData("/api/backend/recovery-plans", recoveryPlanSchema, {
    method: "POST",
    headers: mutationHeaders(),
    body: JSON.stringify(input),
    signal
  });
}

export async function fetchCurrentPlan(signal?: AbortSignal): Promise<RecoveryPlan | null> {
  return requestData("/api/backend/recovery-plans/current", recoveryPlanSchema.nullable(), {
    signal
  });
}

export async function approveRecovery(
  planId: string,
  fingerprint: string,
  signal?: AbortSignal
): Promise<RecoveryPlan> {
  return requestData(`/api/backend/recovery-plans/${planId}/approve`, recoveryPlanSchema, {
    method: "POST",
    headers: mutationHeaders(),
    body: JSON.stringify({ fingerprint }),
    signal
  });
}

export async function rejectRecovery(
  planId: string,
  signal?: AbortSignal
): Promise<RecoveryPlan> {
  return requestData(`/api/backend/recovery-plans/${planId}/reject`, recoveryPlanSchema, {
    method: "POST",
    headers: mutationHeaders(),
    signal
  });
}

export async function executeRecovery(
  planId: string,
  signal?: AbortSignal
): Promise<RecoveryPlan> {
  return requestData(`/api/backend/recovery-plans/${planId}/execute`, recoveryPlanSchema, {
    method: "POST",
    headers: mutationHeaders(),
    signal
  });
}

export async function verifyRecovery(
  planId: string,
  signal?: AbortSignal
): Promise<RecoveryVerification> {
  return requestData(
    `/api/backend/recovery-plans/${planId}/verify`,
    recoveryVerificationSchema,
    { signal }
  );
}

export async function fetchAuditEvents(signal?: AbortSignal): Promise<AuditEvent[]> {
  return requestData("/api/backend/audit-events", z.array(auditEventSchema), { signal });
}

function mutationHeaders(): HeadersInit {
  return {
    "Content-Type": "application/json",
    "X-Demo-Request": "1"
  };
}
