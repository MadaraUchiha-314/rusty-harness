# T14 — prompts, strings and locales

<!-- markdownlint-disable MD013 -->

Every prompt fragment resolves to a file, catalogs match `en`, and a missing key falls back. The web `test:strings` script was corrected in this run: `--dir src` with the config's `src/**` include matched no file, so it had never run a test.

## Assets tests

```sh
cargo nextest run -p rusty_harness --all-features -E 'test(assets_)'
```

Outcome: **passed** in 2s, 2026-10-25, at `0a97f5b`.

```text
Starting 3 tests across 30 binaries (244 tests skipped)
        PASS [   0.006s] (1/3) rusty_harness::assets assets_every_prompt_fragment_resolves_to_a_file
        PASS [   0.007s] (2/3) rusty_harness::assets assets_placeholder_sets_match_en
        PASS [   0.045s] (3/3) rusty_harness::assets assets_no_raw_prompt_literals
     Summary [   0.046s] 3 tests run: 3 passed, 244 skipped
```

## The web string catalog

```sh
bun run test:strings
```

Outcome: **passed** in 6s, 2026-10-25, at `0a97f5b`.

```text
Test Files  1 passed | 6 skipped (7)
      Tests  2 passed | 22 skipped (24)
```
