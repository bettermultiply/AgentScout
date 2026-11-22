import fs from 'node:fs';
import path from 'node:path';
import {spawn} from 'node:child_process';

export function runPath(root, file) {
  const runs = path.join(root, 'runs');
  const ids = fs.existsSync(runs) ? fs.readdirSync(runs).filter(id => fs.statSync(path.join(runs,id)).isDirectory()) : [];
  if (ids.length > 1) throw Error('Fixture expected one run');
  if (file.startsWith('workspace')) return path.join(root,file);
  return path.join(runs, ids[0] ?? 'not-started', file);
}

export function spawnControl(binary, fixtureArgs, options) {
  const [root, piRoot, policy, ...rest] = fixtureArgs;
  const home = path.join(root, 'ascout-home');
  fs.mkdirSync(path.join(home,'.config/ascout'), {recursive:true});
  fs.writeFileSync(path.join(home,'.config/ascout/config.toml'),
    `policy_dir = ${JSON.stringify(path.dirname(path.dirname(policy)))}\nruns_dir = ${JSON.stringify(path.join(root,'runs'))}\ndefault_policy = ${JSON.stringify(path.basename(path.dirname(policy)))}\n`);
  const runtimePath = options.env.ASCOUT_RUNTIME_CONFIG;
  if (runtimePath) {
    const runtime = JSON.parse(fs.readFileSync(runtimePath, 'utf8'));
    const runs = path.join(root,'runs');
    fs.mkdirSync(runs,{recursive:true});
    const configure = config => {
      if (config.filesystem && !config.filesystem.isolated_paths.includes(runs))
        config.filesystem.isolated_paths.push(runs);
    };
    configure(runtime);
    for (const backend of runtime.backends ?? []) configure(backend.config);
    fs.writeFileSync(runtimePath,JSON.stringify(runtime));
  }
  const resume = rest.indexOf('--resume');
  if (resume >= 0) rest.splice(resume,1,'--resume',path.basename(path.dirname(runPath(root,'recovery.json'))));
  // Seed synchronously in the adapter launcher, before Runtime can observe the workspace.
  const launcher = path.join(root, 'runtime-launcher.cjs');
  const adapter = options.env.ASCOUT_RUNTIME_ADAPTER ?? path.join(path.dirname(binary), 'ascout-runtime-adapter');
  fs.writeFileSync(launcher, `#!/usr/bin/env node
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const root = ${JSON.stringify(root)};
const runs = path.join(root, 'runs');
const ids = fs.readdirSync(runs);
if (${resume < 0} && fs.existsSync(path.join(root, 'workspace'))) {
  if (ids.length !== 1) throw Error('Fixture expected one run');
  fs.cpSync(path.join(root, 'workspace'), path.join(runs, ids[0], 'workspace'), {recursive:true});
}
const child = cp.spawn(${JSON.stringify(adapter)}, process.argv.slice(2), {stdio:'inherit'});
for (const signal of ['SIGTERM','SIGINT']) process.on(signal, () => child.kill(signal));
child.on('exit', code => process.exit(code ?? 1));
`, {mode:0o755});
  return spawn(binary,[piRoot,'--cwd',path.join(root,'workspace'),...rest],{...options,env:{...options.env,HOME:home,ASCOUT_RUNTIME_ADAPTER:launcher}});
}
