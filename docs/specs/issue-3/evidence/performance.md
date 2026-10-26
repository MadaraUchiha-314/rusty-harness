# Performance and size evidence (T7)

The NFR targets are measured, not gates (Q13). Linux x64, 2026-10-25, on
`loop/issue-3-m7-ship`: first at task 45, then again at the verification node on
`0a97f5b` (task 51), which is the summary below.

## Summary

| Target | Measured (task 51) | Task 45 | Budget | Verdict |
|---|---|---|---|---|
| Loop overhead per turn, p50 (instant model) | 289.3 µs (95% CI 289.0–289.6) | 259.0 µs | 5 ms | within |
| TUI cold start to the first-message prompt, median of 5 | 24.0 ms | 26.5 ms | 200 ms | within |
| `rusty-harness` binary (profile `dist`) | 19 939 864 bytes | 19 733 944 | 25 000 000 | within |
| `rusty_harness_bg.wasm`, gzip -9 | 1 290 493 bytes (raw 4 072 803) | 1 239 677 | 3 000 000 | within |
| Prompt-cache hit rate, calls 2–4 (`gpt-6.1-sol`) | — | 97% | — (R15.2) | cached (task 45) |

Between the two runs, the loop overhead rose 11% and the binary and wasm module grew
about 1% and 4%. Tasks 53 and 54 landed in between: the plan tools join every model
request, and every message is checked for a command first. That is the likely cause,
but it was not isolated. Every target still has its headroom: the overhead is 17×
under budget.

## Loop overhead

`cargo bench -p rusty_harness --bench loop_overhead`: one `send_message` through the
whole loop (task open, context build, the model call, delivery, the store commit)
against a model that answers at once, so only the harness is measured. Criterion, 100
samples (20k iterations):

| Statistic | Estimate | 95% CI |
|---|---|---|
| median (p50) | 289.3 µs | 289.0–289.6 µs |
| mean | 289.2 µs | 288.9–289.6 µs |

Task 45's run: median 259.0 µs (255.9–261.9), mean 261.8 µs (257.9–267.1).

## Sizes and cold start

`scripts/measure.sh` builds the binary with the `dist` profile
([decision-006](../../../decisions/decision-006.md)) and the npm package, then times the
TUI with `scripts/cold-start.py`: exec to the first-message prompt in a pseudo-terminal
that answers the start-up queries a terminal answers, in a throwaway `HOME` with a
`default` model configured (nothing is sent to a provider).

| Target | Measured | Budget | Verdict |
|---|---|---|---|
| `rusty-harness` (profile dist), bytes | 19939864 | 25000000 | within |
| `rusty_harness_bg.wasm` (gzip -9), bytes | 1290493 (raw 4072803) | 3000000 | within |
| TUI cold start, ms (median of 5) | 24.0 | 200 | within |

(`scripts/measure.sh`'s own table at task 51. At task 45: 19733944, 1239677, 26.5.)

How the binary got there (linux-x64, `--features cli`): 67 579 057 bytes with `release`;
36 454 920 with strip, fat LTO and one codegen unit; 23 663 432 with `opt-level = "s"`;
19 733 816 with `panic = "abort"` too, the `dist` profile. The `Binaries` CI job lists
every platform's size in its step summary and the Linux cold start beside it.

A terminal that never answers the device-status query (`CSI 5 n`) holds the first frame
for 2 s, the graphics probe's timeout (`ratatui-image`); every terminal tried answers it.

## Prompt caching (R15.2)

`cargo run -p rusty_harness --features rt-tokio --example cache_hit_rate` sends four
short questions in one context to OpenAI (`gpt-6.1-sol`, the Responses API, the key by
reference `env:OPENAI_API_KEY`) and reads each call's usage.

With the default system prompt each call's input is about 610 tokens, under OpenAI's
1024-token caching floor, so nothing is cached (0% on every call). That is the expected
outcome for a prompt this short, not a missed breakpoint. With a stable 2.8k-token
system prompt (`RH_CACHE_SYSTEM_PROMPT=file:...`) the prefix the context positions keep
first is served from the cache from the second call on:

| Call | Input tokens | Cached | Hit rate |
|---|---|---|---|
| 1 | 2853 | 0 | 0% |
| 2 | 2862 | 2786 | 97% |
| 3 | 2865 | 2786 | 97% |
| 4 | 2862 | 2786 | 97% |

Overall 8358 of 11442 input tokens cached (73%); calls 2–4: 97%.

Anthropic: no key in this environment, so its `cache_control` breakpoints and
`cache_read_input_tokens` are covered by the client's unit tests
(`streams_text_tool_use_and_usage_with_the_api_key_header`, `snapshot_anthropic_request_body`) and not
measured live. The demo flow's own hit rate is measured with the demo (task 46).

The first live run also found three defects the scripted tests could not see, each fixed
in its own commit on PR #13: dotted tool names the providers refuse (`WireNames`), a
`file:` system prompt that was never loaded, and Chat Completions refusing tools with
reasoning on current OpenAI models (the Responses wire).

## Inbox flood (abuse 8)

Measured by the security sweep (task 48).
