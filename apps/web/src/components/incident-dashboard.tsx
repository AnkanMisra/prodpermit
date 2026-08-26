import type {
  DiagnosticResult,
  AuditEvent,
  IncidentSnapshot,
  LogEvent,
  RecoveryPlan,
  RecoveryVerification,
  ReleaseComparison
} from "@/lib/contracts";
import type { ToolActivity, ToolDescriptor } from "@/lib/webmcp/registry";

const numberFormatter = new Intl.NumberFormat("en-US");

type InvestigationView = {
  comparison: ReleaseComparison | undefined;
  logs: LogEvent[];
  diagnostic: DiagnosticResult | undefined;
};

type WebMcpView = {
  kind: "detecting" | "unsupported" | "supported" | "error";
  tools: ToolDescriptor[];
  activity: ToolActivity[];
  message?: string;
};

type RecoveryView = {
  plan: RecoveryPlan | undefined;
  verification: RecoveryVerification | undefined;
  actionError: string | undefined;
  auditEvents: AuditEvent[];
  onApprove: () => Promise<void>;
  onReject: () => Promise<void>;
};

export function IncidentDashboard({
  snapshot,
  investigation = { comparison: undefined, logs: [], diagnostic: undefined },
  webMcp = { kind: "unsupported", tools: [], activity: [] },
  recovery
}: {
  snapshot: IncidentSnapshot;
  investigation?: InvestigationView;
  webMcp?: WebMcpView;
  recovery?: RecoveryView;
}) {
  const points = chartPoints(snapshot);
  const statusLabel = snapshot.health.status === "critical" ? "Critical" : "Healthy";

  return (
    <main className="dashboard" id="main-content">
      <section className="incident-banner" aria-labelledby="incident-title">
        <div>
          <p className="eyebrow">Active incident</p>
          <h1 id="incident-title">{snapshot.incident.serviceId}</h1>
          <p className="incident-summary">{snapshot.incident.summary}</p>
        </div>
        <div className={`status-pill status-${snapshot.health.status}`} role="status">
          <span aria-hidden="true" className="status-dot" />
          {statusLabel}
        </div>
      </section>

      <section className="metric-grid" aria-label="Service health">
        <Metric label="Error rate" value={`${snapshot.health.errorRatePercent.toFixed(1)}%`} />
        <Metric
          label="P95 latency"
          value={`${numberFormatter.format(snapshot.health.p95LatencyMs)} ms`}
        />
        <Metric
          label="Request rate"
          value={`${numberFormatter.format(snapshot.health.requestRateRps)} rps`}
        />
        <Metric label="Current release" value={snapshot.health.currentRelease} mono />
      </section>

      <div className="dashboard-grid">
        <section className="panel telemetry-panel" aria-labelledby="telemetry-title">
          <div className="panel-heading">
            <div>
              <p className="eyebrow">Last 30 minutes</p>
              <h2 id="telemetry-title">Checkout error rate</h2>
            </div>
            <span className="trend-label">Sharp increase after deploy</span>
          </div>
          <svg
            className="telemetry-chart"
            viewBox="0 0 640 220"
            role="img"
            aria-label="Checkout error rate over time"
          >
            <title>Checkout error rate over time</title>
            <line x1="24" y1="190" x2="616" y2="190" className="chart-axis" />
            <line x1="24" y1="30" x2="24" y2="190" className="chart-axis" />
            <polyline points={points} className="chart-line" />
          </svg>
          <p className="chart-caption">
            Error rate rose from 0.3% to {snapshot.health.errorRatePercent.toFixed(1)}%
            shortly after `release_284`.
          </p>
        </section>

        <section className="panel" aria-labelledby="release-title">
          <div className="panel-heading">
            <div>
              <p className="eyebrow">Deployment context</p>
              <h2 id="release-title">Recent releases</h2>
            </div>
          </div>
          <ol className="release-list">
            {snapshot.releases.map((release) => (
              <li key={release.id} className={`release release-${release.state}`}>
                <div>
                  <span className="release-id">{release.id}</span>
                  <span className="release-state">{releaseLabel(release.state)}</span>
                </div>
                <p>{release.description}</p>
                <code>{release.commitSha}</code>
              </li>
            ))}
          </ol>
        </section>
      </div>

      <div className="investigation-grid">
        <ReleaseComparisonPanel comparison={investigation.comparison} />
        <DiagnosticPanel diagnostic={investigation.diagnostic} />
        <LogsPanel logs={investigation.logs} />
        <ToolInspector webMcp={webMcp} />
      </div>
      {recovery ? <RecoveryPanel recovery={recovery} /> : null}
      {recovery ? <AuditTimeline events={recovery.auditEvents} /> : null}
    </main>
  );
}

function RecoveryPanel({ recovery }: { recovery: RecoveryView }) {
  const plan = recovery.plan;
  return (
    <section className="panel recovery-panel" aria-labelledby="recovery-title">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Human authority boundary</p>
          <h2 id="recovery-title">Recovery plan</h2>
        </div>
        <span className={`plan-status plan-${plan?.status ?? "empty"}`}>
          {plan?.status ?? "Not prepared"}
        </span>
      </div>
      {plan ? (
        <div className="plan-content">
          <div className="plan-route">
            <div>
              <span>Current</span>
              <code>{plan.currentRelease}</code>
            </div>
            <span aria-hidden="true">→</span>
            <div>
              <span>Target</span>
              <code>{plan.targetRelease}</code>
            </div>
          </div>
          <dl className="plan-details">
            <div>
              <dt>Reason</dt>
              <dd>{plan.reason}</dd>
            </div>
            <div>
              <dt>Risk</dt>
              <dd>{plan.riskLevel}</dd>
            </div>
            <div>
              <dt>Expires</dt>
              <dd>{new Date(plan.expiresAt).toLocaleTimeString()}</dd>
            </div>
            <div>
              <dt>Fingerprint</dt>
              <dd><code>{plan.fingerprint.slice(0, 12)}</code></dd>
            </div>
          </dl>
          <ul className="precondition-list">
            {plan.preconditions.map((precondition) => (
              <li key={precondition}>{precondition}</li>
            ))}
          </ul>
          <div className="production-change">
            <span>Production changed</span>
            <strong>{plan.status === "executed" ? "Yes" : "No"}</strong>
          </div>
          {plan.status === "prepared" ? (
            <div className="plan-actions">
              <button type="button" className="secondary-button" onClick={recovery.onReject}>
                Reject
              </button>
              <button type="button" className="primary-button" onClick={recovery.onApprove}>
                Approve exact plan
              </button>
            </div>
          ) : null}
          {recovery.actionError ? <p className="action-error" role="alert">{recovery.actionError}</p> : null}
          {recovery.verification ? (
            <p className="verification-result" role="status">
              Verified {recovery.verification.currentRelease}: {recovery.verification.healthStatus}
            </p>
          ) : null}
        </div>
      ) : (
        <EmptyPanel text="Ask the agent to prepare the safest recovery. No production change has occurred." />
      )}
    </section>
  );
}

function AuditTimeline({ events }: { events: AuditEvent[] }) {
  return (
    <section className="panel audit-panel" aria-labelledby="audit-title">
      <p className="eyebrow">Server record</p>
      <h2 id="audit-title">Audit timeline</h2>
      {events.length ? (
        <ol className="audit-list">
          {events.map((event) => (
            <li key={event.id}>
              <time dateTime={event.recordedAt}>
                {new Date(event.recordedAt).toLocaleTimeString()}
              </time>
              <div>
                <strong>{event.eventType.replaceAll("_", " ")}</strong>
                <p>{event.detail}</p>
              </div>
              <span>{event.outcome}</span>
            </li>
          ))}
        </ol>
      ) : (
        <EmptyPanel text="No recovery events recorded." />
      )}
    </section>
  );
}

function ReleaseComparisonPanel({ comparison }: { comparison: ReleaseComparison | undefined }) {
  return (
    <section className="panel" aria-labelledby="comparison-title">
      <p className="eyebrow">Agent evidence</p>
      <h2 id="comparison-title">Release comparison</h2>
      {comparison ? (
        <div className="diff-list">
          <p className="comparison-route">
            <code>{comparison.baseline.id}</code> to <code>{comparison.candidate.id}</code>
          </p>
          {comparison.configurationDiff.map((difference) => (
            <article className="diff-row" key={difference.key}>
              <div>
                <strong>{difference.key}</strong>
                {difference.suspectedRegression ? <span>Suspected regression</span> : null}
              </div>
              <code>{difference.baselineValue}</code>
              <span aria-hidden="true">→</span>
              <code>{difference.candidateValue}</code>
            </article>
          ))}
        </div>
      ) : (
        <EmptyPanel text="No release comparison has run." />
      )}
    </section>
  );
}

function DiagnosticPanel({ diagnostic }: { diagnostic: DiagnosticResult | undefined }) {
  return (
    <section className="panel" aria-labelledby="diagnostic-title">
      <p className="eyebrow">Safe diagnostic</p>
      <h2 id="diagnostic-title">Database connectivity</h2>
      {diagnostic ? (
        <div className={`diagnostic-result diagnostic-${diagnostic.status}`}>
          <strong>{diagnostic.status === "failed" ? "Failed" : "Passed"}</strong>
          <p>{diagnostic.summary}</p>
          <code>{diagnostic.code}</code>
          <p>{diagnostic.evidence}</p>
        </div>
      ) : (
        <EmptyPanel text="The database diagnostic has not run." />
      )}
    </section>
  );
}

function LogsPanel({ logs }: { logs: LogEvent[] }) {
  return (
    <section className="panel logs-panel" aria-labelledby="logs-title">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Bounded operational data</p>
          <h2 id="logs-title">Structured logs</h2>
        </div>
        <span className="untrusted-label">Untrusted content</span>
      </div>
      {logs.length ? (
        <ol className="log-list">
          {logs.map((event) => (
            <li key={event.id} className={`log-row log-${event.severity}`}>
              <time dateTime={event.recordedAt}>
                {new Date(event.recordedAt).toLocaleTimeString([], {
                  hour: "2-digit",
                  minute: "2-digit"
                })}
              </time>
              <code>{event.code}</code>
              <span>{event.message}</span>
              {event.untrusted ? <strong>External text</strong> : null}
            </li>
          ))}
        </ol>
      ) : (
        <EmptyPanel text="No log query has run." />
      )}
    </section>
  );
}

function ToolInspector({ webMcp }: { webMcp: WebMcpView }) {
  const latest = webMcp.activity.at(-1);
  return (
    <section className="panel tool-panel" aria-labelledby="tools-title">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Capability registry</p>
          <h2 id="tools-title">WebMCP tools</h2>
        </div>
        <span className={`protocol-status protocol-${webMcp.kind}`}>{webMcp.kind}</span>
      </div>
      {webMcp.kind === "unsupported" ? (
        <p className="unsupported-message">
          This browser does not expose WebMCP. Use ChatGPT&apos;s in-app browser or enable
          Chrome&apos;s WebMCP testing flag.
        </p>
      ) : null}
      <ul className="tool-list">
        {webMcp.tools.map((tool) => (
          <li key={tool.name}>
            <code>{tool.name}</code>
            <span>{tool.classification}</span>
          </li>
        ))}
      </ul>
      <div className="activity-line" aria-live="polite">
        <span>Latest invocation</span>
        {latest ? (
          <strong>
            {latest.toolName}: {latest.status}
          </strong>
        ) : (
          <strong>None</strong>
        )}
      </div>
      <p className="capability-note">
        Execution capability absent. Human approval is required before it can exist.
      </p>
    </section>
  );
}

function EmptyPanel({ text }: { text: string }) {
  return <p className="empty-panel">{text}</p>;
}

function Metric({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <article className="metric-card">
      <p>{label}</p>
      <strong className={mono ? "metric-value metric-mono" : "metric-value"}>{value}</strong>
    </article>
  );
}

function chartPoints(snapshot: IncidentSnapshot): string {
  const width = 592;
  const height = 160;
  const maximum = snapshot.telemetry.reduce(
    (current, point) => Math.max(current, point.errorRatePercent),
    20
  );
  const denominator = Math.max(snapshot.telemetry.length - 1, 1);
  return snapshot.telemetry
    .map((point, index) => {
      const x = 24 + (index / denominator) * width;
      const y = 190 - (point.errorRatePercent / maximum) * height;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

function releaseLabel(state: IncidentSnapshot["releases"][number]["state"]): string {
  switch (state) {
    case "healthy_baseline":
      return "Healthy baseline";
    case "deployed_faulty":
      return "Current, suspected regression";
    case "staged":
      return "Staged, unrelated";
    default: {
      const exhaustive: never = state;
      return exhaustive;
    }
  }
}
