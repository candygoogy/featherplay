import { defineConfig } from "astro/config";

// Static output only: the site is a few files on Cloudflare's static assets.
//
// Every script and stylesheet is a file under /_astro/, never inlined into a page, so the
// Content-Security-Policy in public/_headers allows scripts and style sheets from 'self' alone
// (no hashes to keep in step). tests/csp.spec.ts fails if a page grows an inline one.
export default defineConfig({
  site: "https://featherplay.candygoogy.dev",
  output: "static",
  trailingSlash: "ignore",
  build: { inlineStylesheets: "never", format: "file" },
  vite: {
    build: {
      // Scripts and styles are never inlined (the CSP); other assets keep Vite's default.
      assetsInlineLimit: (file) => (/\.(?:m?js|css)$/.test(file) ? false : undefined),
    },
  },
  devToolbar: { enabled: false },
  compressHTML: true,
});
