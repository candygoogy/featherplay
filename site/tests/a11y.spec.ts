import { expect, test } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

// axe, WCAG 2.2 A and AA, on every page in both themes. Each page's results are also written
// as JSON to $A11Y_REPORT_DIR, else to site/test-results/a11y/ (gitignored).
const out = process.env.A11Y_REPORT_DIR || fileURLToPath(new URL("../test-results/a11y", import.meta.url));

for (const theme of ["dark", "light"] as const) {
  for (const path of ["/", "/method", "/credits", "/404"]) {
    test(`axe finds no violations on ${path} (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await page.goto(path);
      await page.waitForLoadState("networkidle");
      const tags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
      // The laptops' screens are pictures of the app at its own 1440 px size, scaled down:
      // their targets shrink with them (WCAG 2.5.8's "essential" presentation). Every other
      // rule runs on them; target size runs on the rest of the page.
      const all = await new AxeBuilder({ page }).withTags(tags).disableRules(["target-size"]).analyze();
      const sizes = await new AxeBuilder({ page }).withRules(["target-size"]).exclude("[data-desktop]").exclude("[data-replica]").analyze();
      const results = { violations: [...all.violations, ...sizes.violations], passes: [...all.passes, ...sizes.passes] };
      try {
        mkdirSync(out, { recursive: true });
        const name = `axe-${path === "/" ? "home" : path.slice(1)}-${theme}.json`;
        writeFileSync(`${out}/${name}`, JSON.stringify({ url: path, theme, violations: results.violations, passes: results.passes.length }, null, 2));
      } catch {}
      const summary = results.violations.map((v) => `${v.id}: ${v.nodes.length} (${v.nodes[0]?.target.join(" ")})`);
      expect(summary).toEqual([]);
    });
  }
}
