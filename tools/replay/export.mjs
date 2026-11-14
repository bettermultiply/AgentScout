#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { loadRecording } from './recording.mjs';
const run=process.argv[2];
if(!run)throw Error('usage: node tools/replay/export.mjs RUN_DIR [OUTPUT]');
const {recording,aborted,skippedError}=loadRecording(run);
fs.writeFileSync(process.argv[3]||path.join(run,'recording.json'),JSON.stringify(recording,null,2)+'\n');
console.error(`assistant records: successful=${recording.length-aborted}, aborted=${aborted}, skipped error=${skippedError}`);
