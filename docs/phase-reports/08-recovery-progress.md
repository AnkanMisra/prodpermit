# Recovery progress

The recovery panel now explains the next action for each server-reported plan status. An ordered Prepare, Approve, Execute, Verify indicator distinguishes approval from execution and execution from verified recovery. Terminal plans show stopped progress. Only passing verification for the displayed executed plan shows completion.

The change is presentation-only. Rust continues to own authorization and state transitions. The current plan ID is available alongside the instructions.

Verification covers the normal progression, terminal states, verification mismatches, and results belonging to another plan, plus the web type check, lint, and production build.
