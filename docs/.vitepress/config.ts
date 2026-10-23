import { defineConfig } from "vitepress";

const repo = "https://github.com/MadaraUchiha-314/rusty-harness";

export default defineConfig({
  title: "rusty-harness",
  description: "A tiny agent harness that's blazing fast.",
  base: "/rusty-harness/",
  cleanUrls: true,
  lastUpdated: true,
  // Work-item specs are the-loop's working records, not site pages.
  srcExclude: ["specs/**", "README.md"],
  // The rustdoc API reference is copied into the build output by CI, after VitePress runs.
  ignoreDeadLinks: [/^\/api\//],
  themeConfig: {
    nav: [
      { text: "Guide", link: "/guide/getting-started" },
      { text: "Development", link: "/development/tech-stack" },
      { text: "Internals", link: "/architecture/architecture" },
      { text: "API reference", link: "/api/rusty_harness/index.html", target: "_blank" },
    ],
    sidebar: [
      {
        text: "Guide",
        items: [
          { text: "Getting started", link: "/guide/getting-started" },
          { text: "Async model", link: "/guide/async-model" },
        ],
      },
      {
        text: "Development",
        items: [
          { text: "Tech stack", link: "/development/tech-stack" },
          { text: "Local development", link: "/development/local-development" },
          { text: "Git hooks and commits", link: "/development/hooks-and-commits" },
          { text: "CI and releases", link: "/development/ci-and-releases" },
        ],
      },
      {
        text: "Internals",
        items: [
          { text: "Architecture", link: "/architecture/architecture" },
          { text: "Capabilities", link: "/capabilities/capabilities" },
          { text: "Decisions", link: "/decisions/decisions" },
          { text: "Learnings", link: "/learnings/learnings" },
        ],
      },
    ],
    socialLinks: [{ icon: "github", link: repo }],
    editLink: { pattern: `${repo}/edit/main/docs/:path`, text: "Edit this page on GitHub" },
    search: { provider: "local" },
    footer: { message: "Released under the Apache-2.0 License." },
  },
});
