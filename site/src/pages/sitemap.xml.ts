// The sitemap: the pages there are. The 404 isn't listed.
import type { APIRoute } from "astro";

const pages = ["/", "/method", "/credits"];

export const GET: APIRoute = ({ site }) => {
  const base = site ?? new URL("https://featherplay.candygoogy.dev");
  const urls = pages.map((p) => `  <url><loc>${new URL(p, base).href}</loc></url>`).join("\n");
  const body = `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls}\n</urlset>\n`;
  return new Response(body, { headers: { "Content-Type": "application/xml; charset=utf-8" } });
};
