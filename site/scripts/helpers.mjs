// The yt-dlp builds the app falls back to when GitHub can't be reached: part of the build,
// before `astro build`.
//   pnpm --dir site run helpers
//
// The app pins each yt-dlp build with its SHA256 in crates/streams/src/sidecars.rs, and its
// second source is this site, at `ytdlp_mirror` (/helpers/yt-dlp/<version>/<file>). This reads
// those pins, downloads each pinned file from GitHub into public/helpers/ (not in git), and
// fails the build when a file doesn't match its pin or is over the 25 MiB a static asset may
// be. A file already there with the right SHA256 is kept. Released builds keep their own
// pins, so after a pin moves the old files stay too: helpers-kept.json lists them (move the
// old entries there in the same commit); any other version is removed. The app checks the
// same pin after its download, so a file here can't change what runs. A pin whose mirror is
// `None` (the Linux builds, at 41 MB over the asset limit) has no copy here.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const SITE = "https://featherplay.candygoogy.dev";
/** Workers static assets serve files up to 25 MiB. */
export const MAX_BYTES = 25 * 1024 * 1024;

/**
 * The yt-dlp pins in sidecars.rs with a mirror (`ytdlp_mirror: Some("…")`): `{ url, sha256, path }`,
 * `path` under the site's root.
 */
export function readPins(source) {
  const re =
    /ytdlp_url:\s*"([^"]+)",\s*ytdlp_sha256:\s*"([0-9a-f]{64})",\s*ytdlp_mirror:\s*Some\(\s*"([^"]+)",?\s*\)/g;
  const pins = [...source.matchAll(re)].map(([, url, sha256, mirror]) => {
    if (!url.startsWith("https://github.com/yt-dlp/yt-dlp/releases/download/")) {
      throw new Error(`not a yt-dlp release on GitHub: ${url}`);
    }
    if (!mirror.startsWith(`${SITE}/helpers/yt-dlp/`)) throw new Error(`not on the site: ${mirror}`);
    const path = mirror.slice(SITE.length + 1);
    if (path.includes("..") || !/^helpers\/yt-dlp\/[\w.-]+\/[\w.-]+\.exe$/.test(path)) {
      throw new Error(`not a helpers path: ${mirror}`);
    }
    return { url, sha256, path };
  });
  if (pins.length === 0) throw new Error("no yt-dlp pins found in sidecars.rs");
  return pins;
}

/** The earlier pins the site keeps serving: `[{ url, sha256, path }]`, checked as `readPins` checks. */
export function readKept(json) {
  return JSON.parse(json).map(({ url, sha256, path }) =>
    readPins(`ytdlp_url: "${url}", ytdlp_sha256: "${sha256}", ytdlp_mirror: Some("${SITE}/${path}")`)[0],
  );
}

/** Throws unless `bytes` are the pinned file. */
export function check(bytes, pin) {
  if (bytes.length > MAX_BYTES) {
    throw new Error(`${pin.path} is ${bytes.length} bytes, over the ${MAX_BYTES}-byte asset limit`);
  }
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (sha256 !== pin.sha256) {
    throw new Error(`${pin.path} has SHA256 ${sha256}, the pin is ${pin.sha256}`);
  }
}

async function main() {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const pins = [
    ...readPins(readFileSync(join(root, "../crates/streams/src/sidecars.rs"), "utf8")),
    ...readKept(readFileSync(join(root, "scripts/helpers-kept.json"), "utf8")),
  ];
  const pub = join(root, "public");
  for (const pin of pins) {
    const dest = join(pub, pin.path);
    if (existsSync(dest)) {
      try {
        check(readFileSync(dest), pin);
        console.log(`helpers: ${pin.path} is there`);
        continue;
      } catch {
        rmSync(dest);
      }
    }
    const resp = await fetch(pin.url, { redirect: "follow" });
    if (!resp.ok) throw new Error(`${pin.url}: HTTP ${resp.status}`);
    const bytes = Buffer.from(await resp.arrayBuffer());
    check(bytes, pin);
    mkdirSync(dirname(dest), { recursive: true });
    writeFileSync(`${dest}.part`, bytes);
    renameSync(`${dest}.part`, dest);
    console.log(`helpers: ${pin.path} (${(bytes.length / 1048576).toFixed(1)} MiB)`);
  }
  // Only the pinned versions stay.
  const versions = join(pub, "helpers/yt-dlp");
  const wanted = new Set(pins.map((p) => p.path.split("/")[2]));
  for (const v of existsSync(versions) ? readdirSync(versions) : []) {
    if (!wanted.has(v)) rmSync(join(versions, v), { recursive: true, force: true });
  }
}

// Run as a script (the build), not when the unit tests import it.
if (!process.env.VITEST) {
  main().catch((e) => {
    console.error(`helpers: ${e.message}`);
    process.exit(1);
  });
}
