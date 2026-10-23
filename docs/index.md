---
layout: home

hero:
  name: rusty-harness
  text: A tiny agent harness that's blazing fast
  tagline: One Rust core, shipped to Rust, Python and JavaScript.
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started
    - theme: alt
      text: Develop locally
      link: /development/local-development

features:
  - title: Rust core
    details: All logic lives in the rusty_harness crate. Every other package is a thin adapter.
  - title: Async everywhere
    details: Public APIs are async and runtime-agnostic — bring tokio, smol, or the browser.
  - title: Three registries, one version
    details: crates.io, PyPI and npm ship the same version from the same commit.
---
