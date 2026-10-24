# CI and releases

## Pull requests — `ci.yml`

| Job | Checks |
|---|---|
| `rust-lint` | fmt, clippy, `cargo check`, `cargo doc` with `-D warnings`, `cargo deny`, `cargo machete` |
| `rust-test` | nextest + doctests on **stable** and on the **MSRV** |
| `wasm-check` | the core for `wasm32-unknown-unknown` — mark this a required status check |
| `wasm` | `wasm-pack test` (Node and headless Chrome), the npm build, vitest, oxlint, tsc, oxfmt |
| `python-lint` | ruff, pyright, and that `rusty_harness.pyi` matches the code |
| `wheels` | abi3 wheels for Linux x86_64 / aarch64, macOS universal2 and Windows, each tested with pytest |
| `docs` | markdownlint and the VitePress build (no deploy) |
| `commitlint` | every commit in the pull request |

The `wasm-check` job is deliberately separate from the `wasm-pack` build, so a runtime
dependency leaking into the core shows up as its own failure.

## Merges to `main` — `release.yml`

```text
push to main ─► ci (everything above)
              ├► release-pr ─ release-plz opens/updates the release PR
              ├► release (env crates-io) ─ publishes when that PR has merged
              │     ├► pypi (env pypi)  ─ wheels + sdist
              │     └► npm  (env npm)   ─ npm publish --provenance
              └► docs ─ GitHub Pages
```

1. **release-plz** keeps a release PR open that bumps the workspace version and
   `CHANGELOG.md` from the conventional commits since the last tag, and runs
   `cargo semver-checks` on the core crate.
2. **Merging the release PR** makes the next run publish: `rusty_harness` goes to
   crates.io and the tag `v<version>` and GitHub release are created. The `-py` and `-wasm`
   crates are `publish = false`.
3. The tag fans out to **PyPI** and **npm** with the same version.

Every registry uses **trusted publishing (OIDC)** — no long-lived tokens are stored:

| Registry | Environment | How |
|---|---|---|
| crates.io | `crates-io` | `rust-lang/crates-io-auth-action` → release-plz `cargo publish -p rusty_harness` |
| PyPI | `pypi` | `pypa/gh-action-pypi-publish` |
| npm | `npm` | `npm publish --provenance` |

### One-time setup

- Create the `crates-io`, `pypi` and `npm` environments in the repository settings.
- Register this repository and `release.yml` as a trusted publisher on each registry.
  crates.io and npm only allow that after the first version exists, so the very first
  publish of each package is a manual one.
- Settings → Pages → Source: **GitHub Actions**.
- Optional: a `RELEASE_PLZ_TOKEN` secret (a fine-grained PAT or GitHub App token). Pull
  requests opened with the default `GITHUB_TOKEN` do not trigger `ci.yml`, so without it
  the release PR's checks have to be started by hand.
