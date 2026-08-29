import { render, screen, within } from "@testing-library/react";
import { describe, expect, test } from "vitest";

import { IncidentDashboard } from "@/components/incident-dashboard";
import {
  incidentSnapshotSchema,
  recoveryPlanSchema,
  recoveryVerificationSchema
} from "@/lib/contracts";

const snapshot = incidentSnapshotSchema.parse({
  session: {
    id: "37f7d3e0-76a0-4887-a543-1b7bbfd1210a",
    createdAt: "2026-08-26T05:00:00Z",
    expiresAt: "2026-08-27T05:00:00Z",
    generation: 1
  },
  incident: {
    id: "inc_checkout_500s",
    serviceId: "checkout-api",
    title: "Checkout requests failing after release_284",
    summary: "Database authentication failures are causing elevated HTTP 500 responses.",
    status: "active",
    startedAt: "2026-08-26T04:50:00Z"
  },
  health: {
    status: "critical",
    errorRatePercent: 18.7,
    p95LatencyMs: 1420,
    requestRateRps: 208,
    currentRelease: "release_284"
  },
  releases: [
    {
      id: "release_283",
      state: "healthy_baseline",
      commitSha: "8f2b9c1",
      description: "Stable checkout release with SCRAM database authentication.",
      deployedAt: "2026-08-23T05:00:00Z"
    },
    {
      id: "release_284",
      state: "deployed_faulty",
      commitSha: "c71a4de",
      description: "Authentication configuration refactor.",
      deployedAt: "2026-08-26T04:48:00Z"
    },
    {
      id: "release_285",
      state: "staged",
      commitSha: "e9802aa",
      description: "Unrelated checkout response metadata change.",
      deployedAt: null
    }
  ],
  telemetry: [
    {
      timestamp: "2026-08-26T04:59:00Z",
      errorRatePercent: 17.1,
      p95LatencyMs: 1304,
      requestRateRps: 210
    },
    {
      timestamp: "2026-08-26T05:00:00Z",
      errorRatePercent: 18.7,
      p95LatencyMs: 1420,
      requestRateRps: 208
    }
  ]
});

describe("IncidentDashboard", () => {
  test("renders the critical checkout incident returned by Rust", () => {
    render(<IncidentDashboard snapshot={snapshot} />);

    expect(screen.getByRole("heading", { name: "checkout-api" })).toBeInTheDocument();
    expect(screen.getByText("Critical")).toBeInTheDocument();
    expect(screen.getByText("18.7%")).toBeInTheDocument();
    expect(screen.getByText("1,420 ms")).toBeInTheDocument();
    expect(
      within(screen.getByRole("region", { name: "Service health" })).getByText(
        "release_284"
      )
    ).toBeInTheDocument();
    expect(
      screen.getByRole("img", { name: "Checkout error rate over time" })
    ).toBeInTheDocument();
  });

  test("pins the telemetry chart to its viewBox so CSS grid cannot stretch it", () => {
    render(<IncidentDashboard snapshot={snapshot} />);

    const chart = screen.getByRole("img", { name: "Checkout error rate over time" });
    expect(chart).toHaveAttribute("viewBox", "0 0 640 220");
    expect(chart).toHaveAttribute("width", "640");
    expect(chart).toHaveAttribute("height", "220");
    expect(chart).toHaveAttribute("preserveAspectRatio", "xMidYMid meet");
  });

  test("shows the exact reviewed plan and persisted verification evidence", () => {
    const snapshot = incidentSnapshotSchema.parse({
      session: {
        id: "37f7d3e0-76a0-4887-a543-1b7bbfd1210a",
        createdAt: "2026-08-26T05:00:00Z",
        expiresAt: "2026-08-27T05:00:00Z",
        generation: 1
      },
      incident: {
        id: "inc_checkout_500s",
        serviceId: "checkout-api",
        title: "Checkout requests restored",
        summary: "The approved rollback restored database connectivity.",
        status: "resolved",
        startedAt: "2026-08-26T04:50:00Z"
      },
      health: {
        status: "healthy",
        errorRatePercent: 0.2,
        p95LatencyMs: 176,
        requestRateRps: 224,
        currentRelease: "release_283"
      },
      releases: [
        {
          id: "release_283",
          state: "healthy_baseline",
          commitSha: "8f2b9c1",
          description: "Stable checkout release.",
          deployedAt: "2026-08-23T05:00:00Z"
        }
      ],
      telemetry: [
        {
          timestamp: "2026-08-26T05:03:00Z",
          errorRatePercent: 0.2,
          p95LatencyMs: 176,
          requestRateRps: 224
        }
      ]
    });
    const planId = "7a43cb85-6604-4a16-9147-7b9f33e567d9";
    const fingerprint = "a".repeat(64);
    const plan = recoveryPlanSchema.parse({
      planId,
      sessionId: snapshot.session.id,
      incidentId: snapshot.incident.id,
      serviceId: snapshot.incident.serviceId,
      currentRelease: "release_284",
      targetRelease: "release_283",
      expectedCurrentRelease: "release_284",
      scenarioGeneration: 1,
      reason: "Rollback the database authentication regression.",
      supportingEvidence: ["log_db_auth_1", "5b8a3bb1-8412-4275-877a-69347ab1800d"],
      riskLevel: "low",
      preconditions: ["The exact approved facts still match."],
      fingerprint,
      createdAt: "2026-08-26T05:01:00Z",
      expiresAt: "2026-08-26T05:11:00Z",
      approvedAt: "2026-08-26T05:02:00Z",
      executedAt: "2026-08-26T05:03:00Z",
      status: "executed"
    });
    const verification = recoveryVerificationSchema.parse({
      planId,
      outcome: { kind: "passed" },
      previousRelease: "release_284",
      currentRelease: "release_283",
      healthStatus: "healthy",
      diagnosticStatus: "passed",
      before: { release: "release_284", evidence: [] },
      after: {
        release: "release_283",
        healthStatus: "healthy",
        incidentStatus: "resolved",
        telemetry: {
          planId,
          serviceId: "checkout-api",
          releaseId: "release_283",
          scenarioGeneration: 1,
          recordedAt: "2026-08-26T05:03:00Z",
          errorRatePercent: 0.2,
          p95LatencyMs: 176,
          requestRateRps: 224
        },
        diagnostic: {
          planId,
          id: "5b8a3bb1-8412-4275-877a-69347ab1800d",
          serviceId: "checkout-api",
          releaseId: "release_283",
          scenarioGeneration: 1,
          kind: "database_connectivity",
          status: "passed",
          code: "DB_CONNECTION_OK",
          summary: "Database connection succeeded.",
          evidence: "The target release negotiated SCRAM authentication.",
          checkedAt: "2026-08-26T05:03:00Z"
        }
      },
      verifiedAt: "2026-08-26T05:04:00Z"
    });

    render(
      <IncidentDashboard
        snapshot={snapshot}
        recovery={{
          plan,
          verification,
          actionError: undefined,
          auditEvents: [],
          onApprove: async () => undefined,
          onReject: async () => undefined
        }}
      />
    );

    expect(screen.getByText("Resolved incident")).toBeInTheDocument();
    expect(screen.getByText(fingerprint)).toBeInTheDocument();
    expect(screen.getByText("log_db_auth_1")).toBeInTheDocument();
    expect(screen.getByText("Recovery verified")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Recovery plan" })).toHaveFocus();
  });
});
