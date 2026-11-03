// The weight budget: run after `astro build`.
//   pnpm --dir site run budget
// First load of `/` (index.html and everything it references, followed through CSS and
// script imports) ≤ 600 KB over the wire; its JavaScript ≤ 60 KB gzipped; the 404 page and
// what it references ≤ 100 KB. Text files are counted gzipped (what a visitor downloads),
// binary files (fonts, images) as they are. Everything referenced counts, lazy images too,
// so the numbers are an upper bound.
import { existsSync, readFileSync, statSync } from "node:fs";
import { join, posix } from "node:path";
import { gzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";

export const BUDGET = { firstLoad: 600 * 1024, js: 60 * 1024, notFound: 100 * 1024 };

const TEXT = /\.(html|css|js|mjs|svg|json|txt|xml|webmanifest)$/i;
const REF =
  /(?:href|src|content)\s*=\s*["']([^"']+)["']|url\(\s*["']?([^"')]+)["']?\s*\)|(?:import|from)\s*\(?\s*["']([^"']+)["']|["'](\/[\w\-./]+\.(?:webp|avif|png|jpe?g|svg|woff2?|js|css))["']/g;

function wire(path) {
  const buf = readFileSync(path);
  return TEXT.test(path) ? gzipSync(buf, { level: 9 }).length : buf.length;
}

function resolveRef(from, ref) {
  if (/^(?:[a-z]+:|\/\/|#|data:)/i.test(ref)) return null;
  const clean = ref.split(/[?#]/)[0];
  if (!clean) return null;
  return clean.startsWith("/")
    ? clean.slice(1)
    : posix.normalize(posix.join(posix.dirname(from), clean));
}

/** Everything `entry` (a path inside `dir`) pulls in, and its size over the wire. */
export function measure(dir, entry) {
  const seen = new Set();
  const queue = [entry];
  while (queue.length) {
    const file = queue.shift();
    if (seen.has(file)) continue;
    const path = join(dir, file);
    if (!existsSync(path) || !statSync(path).isFile()) continue;
    seen.add(file);
    if (!TEXT.test(file)) continue;
    const text = readFileSync(path, "utf8");
    for (const m of text.matchAll(REF)) {
      const ref = m[1] ?? m[2] ?? m[3] ?? m[4];
      const next = ref && resolveRef(file, ref);
      if (next && !seen.has(next)) queue.push(next);
    }
  }
  const files = [...seen];
  const size = (f) => wire(join(dir, f));
  // Scripts inlined into the pages count towards the JavaScript budget as well (their bytes
  // are already in the page's own size).
  const inlineJs = files
    .filter((f) => /\.html$/.test(f))
    .flatMap((f) => [...readFileSync(join(dir, f), "utf8").matchAll(/<script\b(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/gi)])
    .filter((m) => m[1].trim() && !/type=["']application\/ld\+json/i.test(m[0]))
    .reduce((n, m) => n + gzipSync(m[1], { level: 9 }).length, 0);
  return {
    files,
    totalBytes: files.reduce((n, f) => n + size(f), 0),
    jsBytes: inlineJs + files.filter((f) => /\.m?js$/.test(f)).reduce((n, f) => n + size(f), 0),
  };
}

const kb = (n) => `${(n / 1024).toFixed(1)} KB`;

export function check(dir) {
  const lines = [];
  let ok = true;
  const line = (pass, text) => {
    ok &&= pass;
    lines.push(`${pass ? "PASS" : "FAIL"}  ${text}`);
  };
  const index = measure(dir, "index.html");
  line(index.totalBytes <= BUDGET.firstLoad, `first load ${kb(index.totalBytes)} of ${kb(BUDGET.firstLoad)} (${index.files.length} files)`);
  line(index.jsBytes <= BUDGET.js, `JS ${kb(index.jsBytes)} gzipped of ${kb(BUDGET.js)}`);
  if (existsSync(join(dir, "404.html"))) {
    const nf = measure(dir, "404.html");
    line(nf.totalBytes <= BUDGET.notFound, `404 page ${kb(nf.totalBytes)} of ${kb(BUDGET.notFound)}`);
  } else {
    lines.push("SKIP  404 page: not built yet");
  }
  return { ok, lines, index };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const dir = fileURLToPath(new URL("../dist/", import.meta.url));
  if (!existsSync(join(dir, "index.html"))) {
    console.error("No dist/index.html: run `pnpm build` first.");
    process.exit(2);
  }
  const { ok, lines, index } = check(dir);
  for (const l of lines) console.log(l);
  if (process.argv.includes("--files")) {
    for (const f of index.files.sort()) console.log(`      ${kb(wire(join(dir, f))).padStart(9)}  ${f}`);
  }
  process.exit(ok ? 0 : 1);
}
