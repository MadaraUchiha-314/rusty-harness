// Finds the prebuilt binary for this machine: one optional dependency per platform
// (`@rusty-harness/cli-<os>-<cpu>`), of which npm installs only the matching one (Q8).
import { createRequire } from "node:module";

/** The platforms a binary is published for, as `<process.platform>-<process.arch>`. */
export const PLATFORMS = ["linux-x64", "linux-arm64", "darwin-x64", "darwin-arm64", "win32-x64"];

/**
 * The platform package's name for `platform` and `arch`.
 * @param {string} platform
 * @param {string} arch
 * @returns {string}
 */
export function packageName(platform, arch) {
  return `@rusty-harness/cli-${platform}-${arch}`;
}

/**
 * The binary's path inside its platform package.
 * @param {string} platform
 * @returns {string}
 */
export function executable(platform) {
  return platform === "win32" ? "bin/rusty-harness.exe" : "bin/rusty-harness";
}

/**
 * The C library of this Linux process: `glibc` when Node reports a glibc runtime,
 * otherwise `musl` (Alpine and other musl distributions report none).
 * @returns {"glibc" | "musl"}
 */
export function linuxLibc() {
  const report = /** @type {{ header?: { glibcVersionRuntime?: string } } | undefined} */ (
    process.report?.getReport?.()
  );
  return report?.header?.glibcVersionRuntime ? "glibc" : "musl";
}

/**
 * Resolves the installed binary, or explains why there is none.
 * @param {{ platform?: string, arch?: string, libc?: "glibc" | "musl", from?: string }} [options]
 * @returns {{ ok: true, path: string } | { ok: false, message: string }}
 */
export function resolveBinary(options = {}) {
  const platform = options.platform ?? process.platform;
  const arch = options.arch ?? process.arch;
  const name = packageName(platform, arch);
  // The Linux binaries are built against glibc: npm skips them on musl (or installs one
  // that cannot start), so say so instead of advising a reinstall that cannot help.
  if (platform === "linux" && (options.libc ?? linuxLibc()) === "musl") {
    return {
      ok: false,
      message: `rusty-harness: no prebuilt binary for ${platform}-${arch} with musl (the published binaries need glibc); build it with \`cargo install rusty_harness --features cli\`.`,
    };
  }
  if (!PLATFORMS.includes(`${platform}-${arch}`)) {
    return {
      ok: false,
      message: `rusty-harness: no prebuilt binary for ${platform}-${arch}; build it with \`cargo install rusty_harness --features cli\`.`,
    };
  }
  const require = createRequire(options.from ?? import.meta.url);
  try {
    return { ok: true, path: require.resolve(`${name}/${executable(platform)}`) };
  } catch {
    return {
      ok: false,
      message: `rusty-harness: ${name} is not installed; reinstall @rusty-harness/cli without --no-optional (or --omit=optional).`,
    };
  }
}
