#!/usr/bin/env node
// Deep validation consumes an existing run; it never creates a recording run.
process.env.REPLAY_ROUNDS = "3";
process.env.DEEP_ROUNDS ||= "3";
await import("./b3-repeat.ts");
