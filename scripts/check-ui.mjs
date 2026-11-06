// Syntax-checks the UI scripts (no bundler, plain ES modules).
import { readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const dir = new URL("../ui/", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
let failed = 0;
for (const f of readdirSync(dir).filter((f) => f.endsWith(".js"))) {
  try {
    execFileSync(process.execPath, ["--input-type=module", "--check"], { input: (await import("node:fs")).readFileSync(join(dir, f)) });
    console.log(`ok   ${f}`);
  } catch (e) {
    failed++;
    console.error(`FAIL ${f}\n${e.stderr?.toString() ?? e}`);
  }
}
process.exit(failed ? 1 : 0);
