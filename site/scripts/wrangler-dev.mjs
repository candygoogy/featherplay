// Serves dist/ through Cloudflare's own static-assets runtime (wrangler dev, local only), so
// the tests see real status codes and the headers from public/_headers: an unknown path is
// the 404 page with status 404. Sends no metrics. Never deploys.
//
// wrangler's logs and config go under $FEATHERPLAY_WRANGLER_HOME (`logs/`, `config/`) when it's
// set, else under a folder in the OS's temp directory. WRANGLER_LOG_PATH and XDG_CONFIG_HOME,
// when set, win over both.
import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const port = process.argv[2] ?? "8788";
const shell = process.platform === "win32";
if (!existsSync("dist/404.html")) spawnSync("pnpm", ["run", "build"], { stdio: "inherit", shell });
const home = process.env.FEATHERPLAY_WRANGLER_HOME || join(tmpdir(), "featherplay-site-wrangler");
const env = {
  ...process.env,
  WRANGLER_SEND_METRICS: "false",
  WRANGLER_LOG_PATH: process.env.WRANGLER_LOG_PATH ?? join(home, "logs"),
  XDG_CONFIG_HOME: process.env.XDG_CONFIG_HOME ?? join(home, "config"),
};
const child = spawn("pnpm", ["exec", "wrangler", "dev", "--port", port, "--ip", "127.0.0.1"], { stdio: "inherit", env, shell });
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => child.kill(sig));
child.on("exit", (code) => process.exit(code ?? 0));
