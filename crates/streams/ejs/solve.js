// featherplay's n/sig solver, run by deno for each native resolve.
// Loads a player preprocessed by yt-dlp's EJS solver (the file named in the first
// argument), which sets `_result.n` and `_result.sig`, then answers one request read from
// stdin: {"n": [challenges], "sig": [challenges]} -> {"n": {challenge: answer}, "sig": {...}}.
// The player loads while the resolver waits for the player API, so stdin comes later.
const code = await Deno.readTextFile(Deno.args[0]);
const fns = { n: null, sig: null };
let loadError = "";
try {
  Function("_result", code)(fns);
} catch (e) {
  loadError = `: ${e}`;
}
const input = JSON.parse(await new Response(Deno.stdin.readable).text());
const out = {};
for (const type of ["n", "sig"]) {
  out[type] = {};
  for (const challenge of input[type] ?? []) {
    if (typeof fns[type] !== "function") {
      throw new Error(`the player has no ${type} function${loadError}`);
    }
    const answer = fns[type](challenge);
    if (typeof answer !== "string" || !answer) {
      throw new Error(`no ${type} answer for ${challenge}`);
    }
    out[type][challenge] = answer;
  }
}
console.log(JSON.stringify(out));
// The player may have left timers behind.
Deno.exit(0);
