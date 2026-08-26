import { render, screen, within } from "@testing-library/react";
import { describe, expect, test } from "vitest";

import { IncidentDashboard } from "@/components/incident-dashboard";
import { incidentSnapshotSchema } from "@/lib/contracts";

describe("IncidentDashboard", () => {
  test("renders the critical checkout incident returned by Rust", () => {
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
});
