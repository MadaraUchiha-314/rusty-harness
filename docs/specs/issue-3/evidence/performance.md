# Performance and size evidence (T7)

## Artifact sizes

`scripts/measure.sh` on linux-x64, 2026-10-25, at the task 44 commit (profile `dist`,
[decision-006](../../../decisions/decision-006.md)):

| Artifact | Bytes | Budget | Verdict |
|---|---|---|---|
| `rusty-harness` (profile dist) | 19733944 | 25000000 | within |
| `rusty_harness_bg.wasm` (gzip -9) | 1217858 (raw 3825337) | 3000000 | within |

Every platform's binary is also measured, and held to the same budget, by the
`Binaries` CI job (its step summary lists the bytes per platform).
