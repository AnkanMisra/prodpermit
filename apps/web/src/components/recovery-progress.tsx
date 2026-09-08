import type { RecoveryPlan, RecoveryVerification } from "@/lib/contracts";

type Props = {
  plan: Pick<RecoveryPlan, "status" | "planId"> | undefined;
  verification: Pick<RecoveryVerification, "planId" | "outcome"> | undefined;
};

const stages = ["Prepare", "Approve", "Execute", "Verify"];
const guidance = {
  prepared: { step: 1, title: "Awaiting your approval", detail: 'Review the evidence and fingerprint below, then click "Approve exact plan". Preparation has not changed the active release.' },
  approved: { step: 2, title: "Approved. Ask the agent to execute", detail: "Ask the agent to call execute_approved_recovery with the plan ID below. Approval alone does not perform the rollback." },
  executing: { step: 2, title: "Recovery is executing", detail: "Wait for the execution result before requesting verification." },
  executed: { step: 3, title: "Executed. Verify the recovery", detail: "Ask the agent to call verify_recovery with the plan ID below. A healthy dashboard alone is not the final verification." },
  rejected: { step: -1, title: "Plan rejected", detail: "Ask the agent to review the evidence and prepare a new recovery plan." },
  expired: { step: -1, title: "Plan expired", detail: "Ask the agent to gather current evidence and prepare a new plan for your approval." },
  invalidated: { step: -1, title: "Plan no longer valid", detail: "The recovery conditions changed. Ask the agent to inspect the incident again and prepare a new plan." }
} satisfies Record<RecoveryPlan["status"], { step: number; title: string; detail: string }>;

export function RecoveryProgress({ plan, verification }: Props) {
  const result = plan?.status === "executed" && verification?.planId === plan.planId
    ? verification : undefined;
  const current = result
    ? result.outcome.kind === "passed"
      ? { step: 4, title: "Recovery complete", detail: "Verification passed. Review the audit timeline below. Use Reset scenario only when you want to start a new demo." }
      : { step: 3, title: "Verification needs attention", detail: "The verification result does not match the expected recovery. Review the mismatch details below before taking further action." }
    : plan ? guidance[plan.status]
      : { step: 0, title: "Start with the evidence", detail: "Ask the agent to inspect the incident, compare releases, query logs, and run the database diagnostic. Then ask it to prepare a recovery plan." };

  return (
    <div className="recovery-progress">
      <ol aria-label="Recovery progress" className="recovery-stages">
        {stages.map((stage, index) => (
          <li key={stage} aria-current={current.step === index ? "step" : undefined}>
            <span>{index + 1}. {stage}</span>
            <small>{current.step < 0 ? "Stopped" : index < current.step ? "Complete" : index === current.step ? "Current step" : "Pending"}</small>
          </li>
        ))}
      </ol>
      <div aria-live="polite" aria-atomic="true">
        <h3>{current.title}</h3>
        <p>{current.detail}</p>
      </div>
      {plan ? <p className="recovery-plan-id">Plan ID: <code>{plan.planId}</code></p> : null}
    </div>
  );
}
