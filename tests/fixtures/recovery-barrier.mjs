// Hold the sibling at the execution boundary, after Pi's real before-tool
// hook/checkpoint, without imposing an ordering rule on the Runtime backend.
import fs from 'node:fs';
import {AscProxy} from '../../instrumentation/pi/asc-proxy.ts';
const execute=AscProxy.prototype.execute;
AscProxy.prototype.execute=async function(branch,operation,tool,args,context,signal,output){
    if(tool==='nle_action'&&args.action===1){
        const release=process.env.RECOVERY_RELEASE;
        fs.writeFileSync(release+'.pending','');
        while(!fs.existsSync(release)){
            if(signal?.aborted){fs.writeFileSync(release+'.cancelled','');throw Error('Fixture sibling cancelled at execution boundary');}
            await new Promise(resolve=>setTimeout(resolve,5));
        }
    }
    return execute.call(this,branch,operation,tool,args,context,signal,output);
};
