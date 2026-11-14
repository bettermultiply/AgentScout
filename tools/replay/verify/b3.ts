#!/usr/bin/env node
// One full control-plane/Pi/NLE replay, starting from an existing native run.
process.env.REPLAY_ROUNDS = "1";
await import("./b3-repeat.ts");
