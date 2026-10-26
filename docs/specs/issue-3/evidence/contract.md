# T3 — contracts

<!-- markdownlint-disable MD013 -->

The shape of every public interface: the schema and payload contracts, the official TypeScript A2A SDK as the client, and the generated Python stub.

## Contract tests

```sh
cargo nextest run -p rusty_harness --all-features -E 'test(contract_)'
```

Outcome: **passed** in 34s, 2026-10-25, at `0a97f5b`.

```text
Starting 8 tests across 30 binaries (239 tests skipped)
        PASS [   0.011s] (1/8) rusty_harness::assets contract_strings_catalogs_match_en
        PASS [   0.014s] (2/8) rusty_harness::assets contract_prompt_files_match_registry
        PASS [   0.014s] (3/8) rusty_harness::contract_agent_trait contract_agent_trait_matches_sherma_shape
        PASS [   0.017s] (4/8) rusty_harness::contract_hook_catalogue contract_hook_catalogue_covers_sherma_points
        PASS [   0.009s] (5/8) rusty_harness::contract_hook_catalogue contract_hook_catalogue_lists_every_point_of_the_design_table
        PASS [   0.006s] (6/8) rusty_harness::contract_module_tree contract_module_tree_matches_design
        PASS [   0.066s] (7/8) rusty_harness::contract_a2a_extensions contract_a2a_extension_schemas
        PASS [   0.106s] (8/8) rusty_harness::contract_a2a_extensions contract_extension_payloads_match_their_schemas
     Summary [   0.110s] 8 tests run: 8 passed, 239 skipped
```

## The official TypeScript SDK against the server binary

```sh
cargo build -p rusty_harness --features cli --bin rusty-harness && bun run --cwd web test:contract
```

Outcome: **passed** in 84s, 2026-10-25, at `0a97f5b`.

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 21s
 Test Files  1 passed (1)
      Tests  2 passed (2)
```

## The Python stub is up to date

```sh
export PYO3_PYTHON=$PWD/.venv/bin/python; export LD_LIBRARY_PATH=$($PYO3_PYTHON -c 'import sysconfig; print(sysconfig.get_config_var("LIBDIR"))'); cargo run -p rusty-harness-py --bin stub_gen && git diff --exit-code -- rusty_harness.pyi
```

Outcome: **passed** in 20s, 2026-10-25, at `0a97f5b`. `git diff --exit-code` on `rusty_harness.pyi` after regeneration: no difference.

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 20.45s
     Running `<repo>/target/debug/stub_gen`
```
