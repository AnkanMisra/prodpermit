"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { IncidentDashboard } from "@/components/incident-dashboard";
import {
  approveRecovery,
  compareReleases,
  createOrResumeSession,
  executeRecovery,
  fetchAuditEvents,
  fetchCurrentIncident,
  fetchCurrentRecovery,
  prepareRecovery,
  queryLogs,
  rejectRecovery,
  resetSession,
  runDiagnostic,
  verifyRecovery
} from "@/lib/api";
import type {
  DiagnosticResult,
  AuditEvent,
  CurrentRecovery,
  IncidentSnapshot,
  LogEvent,
  RecoveryPlan,
  RecoveryVerification,
  ReleaseComparison
} from "@/lib/contracts";
import { createInvestigationTools } from "@/lib/webmcp/investigation-tools";
import { createExecutionTool, createRecoveryTools } from "@/lib/webmcp/recovery-tools";
import {
  WebMcpRegistry,
  type ToolActivity,
  type ToolDescriptor
} from "@/lib/webmcp/registry";

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; snapshot: IncidentSnapshot }
  | { kind: "error"; message: string };

type WebMcpState =
  | { kind: "detecting"; tools: ToolDescriptor[]; activity: ToolActivity[] }
  | { kind: "unsupported"; tools: ToolDescriptor[]; activity: ToolActivity[] }
  | { kind: "supported"; tools: ToolDescriptor[]; activity: ToolActivity[] }
  | { kind: "error"; tools: ToolDescriptor[]; activity: ToolActivity[]; message: string };

export function ControlRoom() {
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const [comparison, setComparison] = useState<ReleaseComparison>();
  const [logs, setLogs] = useState<LogEvent[]>([]);
  const [diagnostic, setDiagnostic] = useState<DiagnosticResult>();
  const [plan, setPlan] = useState<RecoveryPlan>();
  const [executionCapability, setExecutionCapability] = useState<
    CurrentRecovery["executionCapability"]
  >({ kind: "absent", reason: "no_plan" });
  const [verification, setVerification] = useState<RecoveryVerification>();
  const [actionError, setActionError] = useState<string>();
  const [auditEvents, setAuditEvents] = useState<AuditEvent[]>([]);
  const registryRef = useRef<WebMcpRegistry | null>(null);
  const sessionEpochRef = useRef(0);
  const [webMcp, setWebMcp] = useState<WebMcpState>({
    kind: "detecting",
    tools: [],
    activity: []
  });

  const refreshRecoveryState = useCallback(
    async (signal?: AbortSignal, epoch = sessionEpochRef.current) => {
      const [snapshot, recovery, events] = await Promise.all([
        fetchCurrentIncident(signal),
        fetchCurrentRecovery(signal),
        fetchAuditEvents(signal)
      ]);
      if (epoch !== sessionEpochRef.current) {
        return;
      }
      setState({ kind: "ready", snapshot });
      setPlan(recovery.plan ?? undefined);
      setExecutionCapability(recovery.executionCapability);
      setAuditEvents(events);
    },
    []
  );

  useEffect(() => {
    const controller = new AbortController();
    void createOrResumeSession(controller.signal)
      .then(async (snapshot) => {
        setState({ kind: "ready", snapshot });
        const recovery = await fetchCurrentRecovery(controller.signal);
        setPlan(recovery.plan ?? undefined);
        setExecutionCapability(recovery.executionCapability);
        setAuditEvents(await fetchAuditEvents(controller.signal));
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") {
          return;
        }
        setState({
          kind: "error",
          message: error instanceof Error ? error.message : "The control room could not load."
        });
      });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    const modelContext = document.modelContext;
    if (!modelContext) {
      let active = true;
      queueMicrotask(() => {
        if (active) {
          setWebMcp({ kind: "unsupported", tools: [], activity: [] });
        }
      });
      return () => {
        active = false;
      };
    }

    const registry = new WebMcpRegistry({
      modelContext,
      onActivity: (event) =>
        setWebMcp((current) => ({
          ...current,
          activity: [...current.activity, event].slice(-10)
        })),
      onToolsChanged: (tools) =>
        setWebMcp((current) => ({ ...current, kind: "supported", tools }))
    });
    registryRef.current = registry;
    const registrations = [
      ...createInvestigationTools({
        inspectIncident: async (_input, signal) => {
          const snapshot = await fetchCurrentIncident(signal);
          setState({ kind: "ready", snapshot });
          return toolSuccess("Inspected the active checkout incident.", snapshot);
        },
        compareReleases: async (input, signal) => {
          const result = await compareReleases(input, signal);
          setComparison(result);
          return toolSuccess(
            `Compared ${input.baselineRelease} with ${input.candidateRelease}.`,
            result
          );
        },
        queryLogs: async (input, signal) => {
          const result = await queryLogs(input, signal);
          setLogs(result);
          return toolSuccess(`Returned ${result.length} bounded log events.`, result);
        },
        runDiagnostic: async (_input, signal) => {
          const result = await runDiagnostic(signal);
          setDiagnostic(result);
          return toolSuccess(`Database connectivity diagnostic ${result.status}.`, result);
        }
      }),
      ...createRecoveryTools({
        prepareRecovery: async (input, signal) => {
          const result = await prepareRecovery(input, signal);
          await refreshRecoveryState(signal);
          setActionError(undefined);
          return toolSuccess("Prepared recovery. Production state did not change.", result);
        },
        verifyRecovery: async (input, signal) => {
          const result = await verifyRecovery(input.planId, signal);
          setVerification(result);
          await refreshRecoveryState(signal);
          return toolSuccess("Verified healthy recovery state.", result);
        }
      })
    ];
    void Promise.all(
      registrations.map(({ tool, classification }) => registry.register(tool, classification))
    ).catch((error: unknown) => {
      setWebMcp((current) => ({
        ...current,
        kind: "error",
        message: error instanceof Error ? error.message : "WebMCP registration failed."
      }));
    });
    return () => {
      registry.dispose();
      registryRef.current = null;
    };
  }, [refreshRecoveryState]);

  useEffect(() => {
    const registry = registryRef.current;
    if (!registry) {
      return;
    }
    if (executionCapability.kind !== "available") {
      registry.unregister("execute_approved_recovery");
      return;
    }
    const capability = executionCapability;
    const execution = createExecutionTool(async (input, signal) => {
      if (input.planId !== capability.planId) {
        return {
          ok: false,
          error: {
            code: "PLAN_ID_MISMATCH",
            message: "This capability is bound to a different approved plan.",
            retryable: false
          }
        };
      }
      const executed = await executeRecovery(input.planId, signal);
      await refreshRecoveryState(signal);
      return toolSuccess("Executed the exact approved recovery plan.", executed);
    });
    void registry
      .register(execution.tool, execution.classification, capability.fingerprint)
      .catch((error: unknown) => {
        setActionError(
          error instanceof Error ? error.message : "Execution capability registration failed."
        );
      });
    const delay = Math.max(0, new Date(capability.expiresAt).getTime() - Date.now());
    const expiryTimer = window.setTimeout(() => {
      registry.unregister("execute_approved_recovery");
      void refreshRecoveryState();
    }, delay);
    return () => {
      window.clearTimeout(expiryTimer);
      registry.unregister("execute_approved_recovery");
    };
  }, [executionCapability, refreshRecoveryState]);

  async function approvePlan() {
    if (!plan) {
      return;
    }
    try {
      setActionError(undefined);
      await approveRecovery(plan.planId, plan.fingerprint);
      await refreshRecoveryState();
    } catch (error: unknown) {
      setActionError(error instanceof Error ? error.message : "Plan approval failed.");
    }
  }

  async function rejectPlan() {
    if (!plan) {
      return;
    }
    try {
      setActionError(undefined);
      await rejectRecovery(plan.planId);
      await refreshRecoveryState();
    } catch (error: unknown) {
      setActionError(error instanceof Error ? error.message : "Plan rejection failed.");
    }
  }

  async function resetDemo() {
    try {
      setActionError(undefined);
      sessionEpochRef.current += 1;
      registryRef.current?.unregister("execute_approved_recovery");
      await resetSession();
      setComparison(undefined);
      setLogs([]);
      setDiagnostic(undefined);
      setVerification(undefined);
      await refreshRecoveryState(undefined, sessionEpochRef.current);
    } catch (error: unknown) {
      setActionError(error instanceof Error ? error.message : "Scenario reset failed.");
    }
  }

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to incident
      </a>
      <header className="site-header">
        <div>
          <p className="product-mark">recovery-control-room</p>
          <p className="product-subtitle">Browser-native incident recovery workspace</p>
        </div>
        <div className="header-signals">
          <span className="signal-chip">{webMcpLabel(webMcp.kind)}</span>
          <span className="signal-chip signal-critical">
            {state.kind === "ready" && state.snapshot.incident.status === "resolved"
              ? "Incident resolved"
              : "Incident active"}
          </span>
          <button type="button" className="reset-button" onClick={resetDemo}>
            Reset scenario
          </button>
        </div>
      </header>
      <LoadContent
        state={state}
        comparison={comparison}
        logs={logs}
        diagnostic={diagnostic}
        plan={plan}
        verification={verification}
        actionError={actionError}
        auditEvents={auditEvents}
        webMcp={webMcp}
        approvePlan={approvePlan}
        rejectPlan={rejectPlan}
      />
    </div>
  );
}

function LoadContent(input: {
  state: LoadState;
  comparison: ReleaseComparison | undefined;
  logs: LogEvent[];
  diagnostic: DiagnosticResult | undefined;
  plan: RecoveryPlan | undefined;
  verification: RecoveryVerification | undefined;
  actionError: string | undefined;
  auditEvents: AuditEvent[];
  webMcp: WebMcpState;
  approvePlan: () => Promise<void>;
  rejectPlan: () => Promise<void>;
}) {
  switch (input.state.kind) {
    case "loading":
      return (
        <main className="center-state" id="main-content" aria-live="polite">
          <div className="loading-line" aria-hidden="true" />
          <h1>Opening an isolated incident session</h1>
          <p>Loading the deterministic checkout failure from the Rust service.</p>
        </main>
      );
    case "ready":
      return (
        <IncidentDashboard
          snapshot={input.state.snapshot}
          investigation={{
            comparison: input.comparison,
            logs: input.logs,
            diagnostic: input.diagnostic
          }}
          webMcp={input.webMcp}
          recovery={{
            plan: input.plan,
            verification: input.verification,
            actionError: input.actionError,
            auditEvents: input.auditEvents,
            onApprove: input.approvePlan,
            onReject: input.rejectPlan
          }}
        />
      );
    case "error":
      return (
        <main className="center-state error-state" id="main-content" role="alert">
          <p className="eyebrow">Session unavailable</p>
          <h1>The incident could not be loaded</h1>
          <p>{input.state.message}</p>
          <button type="button" onClick={() => window.location.reload()}>
            Try again
          </button>
        </main>
      );
    default: {
      const exhaustive: never = input.state;
      return exhaustive;
    }
  }
}

function toolSuccess<T>(summary: string, data: T) {
  return { ok: true, summary, data } as const;
}

function webMcpLabel(kind: WebMcpState["kind"]): string {
  switch (kind) {
    case "detecting":
      return "Detecting WebMCP";
    case "supported":
      return "WebMCP supported";
    case "unsupported":
      return "WebMCP unsupported";
    case "error":
      return "WebMCP error";
    default: {
      const exhaustive: never = kind;
      return exhaustive;
    }
  }
}
