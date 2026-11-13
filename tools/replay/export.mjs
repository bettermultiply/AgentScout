#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { loadRecording } from '../../agent-access/replay/recording.mjs';
const run=process.argv[2];
if(!run)throw Error('usage: node tools/replay/export.mjs RUN_DIR [OUTPUT]');
const {recording,skippedAborted,skippedError}=loadRecording(run);
fs.writeFileSync(process.argv[3]||path.join(run,'recording.json'),JSON.stringify(recording,null,2)+'\n');
console.error(`skipped assistant records: aborted=${skippedAborted}, error=${skippedError}, total=${skippedAborted+skippedError}`);
