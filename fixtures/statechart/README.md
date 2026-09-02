# Statechart semantic fixtures

These JSON fixtures name the stable semantic rule IDs they exercise and drive the native
Statechart conformance test. Rule definitions live in
`docs/plan/stage-4-statecharts.md`. `parallel-history-v0.json` covers ordered entry/exit, a Flow
await, commit-before-dispatch Effects, FIFO internal events, parallel selection, shallow/deep
history, completion propagation, varying slice budgets, and final-state stabilization.
