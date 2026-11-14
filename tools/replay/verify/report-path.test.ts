import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";
import test from "node:test";
import { fileURLToPath } from "node:url";

const servePath = fileURLToPath(new URL("../serve.ts", import.meta.url));

for (const name of ["recording.json", "capture.json", "report.json"]) {
  test(`serve preserves ${name} and writes a separate default report`, async t => {
    const root = fs.mkdtempSync("/tmp/opencode/replay-report-");
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    const source = path.join(root, name), before = "[]\n";
    fs.writeFileSync(source, before);
    const child = spawn(process.execPath, [servePath, source, "0"], {
      env: { ...process.env, REPLAY_REPORT: "", REPLAY_REQUEST_LOG: "" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stderr = "";
    child.stderr.on("data", chunk => stderr += chunk);
    const exited = once(child, "exit");
    t.after(async () => { if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL"); await exited; });
    await Promise.race([once(child.stdout, "data"), exited.then(() => { throw Error(`serve exited before ready: ${stderr}`); })]);
    child.kill("SIGTERM");
    assert.equal((await exited)[0], 0);
    assert.equal(fs.readFileSync(source, "utf8"), before, "recording source was overwritten");
    const report = path.join(root, name === "recording.json" ? "report.json" : `${name}.report.json`);
    assert.equal(JSON.parse(fs.readFileSync(report, 'utf8')).total_requests, 0);
  });
}

for (const alias of ["same", "symlink", "hardlink"]) {
  test(`serve rejects an explicit ${alias} report pointing to the source`, async t => {
    const root = fs.mkdtempSync("/tmp/opencode/replay-report-");
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    const source = path.join(root, "capture.json"), before = "[]\n";
    fs.writeFileSync(source, before);
    const report = alias === "same" ? source : path.join(root, "alias.json");
    if (alias === "symlink") fs.symlinkSync(source, report);
    if (alias === "hardlink") fs.linkSync(source, report);
    const child = spawn(process.execPath, [servePath, source, "0"], {
      env: { ...process.env, REPLAY_REPORT: report, REPLAY_REQUEST_LOG: "" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stderr = "";
    child.stderr.on("data", chunk => stderr += chunk);
    const exited = once(child, "exit");
    // An unsafe server may start; shut it down so the old behavior fails promptly.
    child.stdout.on("data", () => child.kill("SIGTERM"));
    t.after(async () => { if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL"); await exited; });
    const [code] = await exited;
    assert.equal(fs.readFileSync(source, "utf8"), before, "recording source was overwritten");
    assert.notEqual(code, 0);
    assert.match(stderr, /report must not overwrite the recording source/);
  });
}
