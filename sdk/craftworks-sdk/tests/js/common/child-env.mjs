// THE ONE ENVIRONMENT A TEST'S CHILD PROCESS RUNS IN (sdk#469; GATE_OWNERS_PER_PR's #452 before it): this suite runs
// under `./gate.sh`, and every `GATE_*` knob the OUTER run was given -- a base ref, a changed-file list, a disk floor,
// the batch's owners flag -- would otherwise steer every gate, helper or tool a test starts. Twice that shipped: an
// outer `GATE_PR_BASE=FETCH_HEAD` broke a child gate in a scratch worktree (no FETCH_HEAD there) and npm lost 34
// tests. The DISK GUARD's knobs (`DISK_GUARD`, `DISK_GUARD_*`) are the outer run's too: an outer guard override would
// steer a child's disk check (sdk#461's red was a child's guard). So a child gets the live environment WITHOUT
// `GATE_*` and `DISK_GUARD*`, plus exactly what its test names.
// tests/js/child-env.test.mjs holds that no test file spreads `process.env` itself.
export const childEnv = (env = {}) => ({
  ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GATE_") && !k.startsWith("DISK_GUARD"))),
  ...env,
});
