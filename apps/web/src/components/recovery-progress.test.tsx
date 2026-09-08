import { render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import { recoveryVerificationSchema } from "@/lib/contracts";
import { RecoveryProgress } from "./recovery-progress";

const firstId = recoveryVerificationSchema.shape.planId.parse("11111111-1111-4111-8111-111111111111");
const secondId = recoveryVerificationSchema.shape.planId.parse("22222222-2222-4222-8222-222222222222");

test("guides the operator through approval, execution, and verification", () => {
  const { rerender } = render(<RecoveryProgress plan={undefined} verification={undefined} />);
  expect(screen.getByText("Start with the evidence")).toBeVisible();
  rerender(<RecoveryProgress plan={{ planId: firstId, status: "prepared" }} verification={undefined} />);
  expect(screen.getByText("Awaiting your approval")).toBeVisible();
  expect(screen.getByText(/Preparation has not changed/)).toBeVisible();
  rerender(<RecoveryProgress plan={{ planId: firstId, status: "approved" }} verification={undefined} />);
  expect(screen.getByText(/Approval alone does not perform/)).toBeVisible();
  rerender(<RecoveryProgress plan={{ planId: firstId, status: "executed" }} verification={undefined} />);
  expect(screen.getByText("Executed. Verify the recovery")).toBeVisible();
  rerender(<RecoveryProgress plan={{ planId: firstId, status: "executed" }} verification={{ planId: firstId, outcome: { kind: "passed" } }} />);
  expect(screen.getByText("Recovery complete")).toBeVisible();
});

test.each(["rejected", "expired", "invalidated"] as const)("stops progress for a %s plan", (status) => {
  render(<RecoveryProgress plan={{ planId: firstId, status }} verification={undefined} />);
  expect(screen.getAllByText("Stopped")).toHaveLength(4);
  expect(document.querySelector('[aria-current="step"]')).toBeNull();
});

test("ignores verification from another plan", () => {
  render(<RecoveryProgress plan={{ planId: secondId, status: "executed" }} verification={{ planId: firstId, outcome: { kind: "passed" } }} />);
  expect(screen.queryByText("Recovery complete")).toBeNull();
  expect(screen.getByText("Executed. Verify the recovery")).toBeVisible();
});

test("shows a mismatch without claiming recovery complete", () => {
  render(<RecoveryProgress plan={{ planId: firstId, status: "executed" }} verification={{ planId: firstId, outcome: { kind: "mismatch", mismatches: ["release"] } }} />);
  expect(screen.getByText("Verification needs attention")).toBeVisible();
  expect(screen.queryByText("Recovery complete")).toBeNull();
});
