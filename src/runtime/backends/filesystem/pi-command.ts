// Runtime-side adapter reuses Pi's tool formatting without an Agent session.
import {createRequire} from 'node:module';
import {resolve} from 'node:path';
import {filesystemTools,stopFilesystemWriters} from './pi-tools.ts';
const [directory,environment,root]=process.argv.slice(2);
const require=createRequire(resolve(root,'package.json'));
let input='';
for await(const chunk of process.stdin)input+=chunk;
const request=JSON.parse(input);
const tool=filesystemTools(directory,environment,request.cwd,require(resolve(root,'packages/coding-agent/src/core/tools/index.ts'))).find(tool=>tool.name===request.tool);
if(!tool)throw Error('Unsupported filesystem tool');
const controller=new AbortController();
process.on('SIGTERM',()=>controller.abort());
try {
    const result=await tool.execute(request.operation_id,request.arguments,controller.signal,value=>process.stdout.write(JSON.stringify({event:'output',value})+'\n'));
    process.stdout.write(JSON.stringify({event:'result',value:{result,evidence:null}})+'\n');
} finally {
    if(controller.signal.aborted)await stopFilesystemWriters(directory,environment);
}
