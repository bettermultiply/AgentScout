// Pi sessions remain in their host process; every built-in file operation uses
// the plan of its own Branch. A fresh mount per operation permits baseline
// replacement between tasks without touching any live overlay.
import {readFileSync} from 'node:fs';
import {spawn} from 'node:child_process';
import {AsyncLocalStorage} from 'node:async_hooks';
import {join} from 'node:path';

// This process is PID 1 in a private PID namespace. Reap *all* descendants,
// including background children that closed stdio or changed process groups.
const supervisor = `import os,sys,subprocess
p=subprocess.Popen(sys.argv[1:])
code=None
while True:
 try:
  pid,status=os.wait()
  if pid==p.pid: code=os.waitstatus_to_exitcode(status)
 except ChildProcessError: break
sys.exit(code if code is not None and code>=0 else 1)
`;

export function filesystemTools(directory, branch, cwd, sdk) {
    if(!directory)return [];
    const context=new AsyncLocalStorage();
    let tail=Promise.resolve();
    const exec=(command,cwd,options={})=>{
        const signal=options.signal??context.getStore();
        const running=tail.then(()=>run(command,cwd,{...options,signal}));
        tail=running.catch(()=>{});
        return running;
    };
    const run=(command,cwd,{onData,signal,timeout,env,input}={})=>new Promise((resolve,reject)=>{
        signal??=context.getStore();
        if(signal?.aborted){reject(Error('Tool cancelled'));return}
        let plan;
        try{plan=JSON.parse(readFileSync(join(directory,branch+'.json'),'utf8'))}catch(error){reject(error);return}
        const child=spawn(plan.program,[...plan.args,'--chdir',cwd,'--','python3','-c',supervisor,'/bin/bash','-c',command],{env:env??process.env,stdio:[input===undefined?'ignore':'pipe','pipe','pipe'],detached:true});
        // Contents travel only through the pipe into the isolated process, never
        // argv or a host temporary file. Wait for close even if the reader exits
        // early: the namespace supervisor must finish reaping descendants first.
        let inputError;
        if(child.stdin){child.stdin.on('error',error=>{inputError=error});child.stdin.end(input)}
        child.stdout.on('data',data=>onData?.(data));child.stderr.on('data',data=>onData?.(data));
        let timedOut=false;
        const stop=()=>{try{process.kill(-child.pid,'SIGKILL')}catch(error){if(error.code!=='ESRCH')throw error}};
        const timer=timeout?setTimeout(()=>{timedOut=true;stop()},timeout*1000):undefined;
        signal?.addEventListener('abort',stop,{once:true});
        // Abort may arrive between the pre-spawn check and listener setup.
        if(signal?.aborted)stop();
        child.once('error',reject);
        child.once('close',code=>{clearTimeout(timer);signal?.removeEventListener('abort',stop);if(signal?.aborted||timedOut)reject(Error(timedOut?'Tool timed out':'Tool cancelled'));else if(inputError)reject(inputError);else resolve({exitCode:code})});
    });
    const python=async(code,args,input=undefined)=>{
        const quote=value=>"'"+String(value).replaceAll("'","'\\''")+"'";
        const chunks=[];
        const result=await exec('python3 -c '+quote(code)+' '+args.map(quote).join(' '),cwd,{input,onData:data=>chunks.push(data)});
        const output=Buffer.concat(chunks);
        if(result.exitCode!==0)throw Error(output.toString());
        return output;
    };
    const readFile=path=>python('import sys,base64; print(base64.b64encode(open(sys.argv[1],"rb").read()).decode())',[path]).then(data=>Buffer.from(data.toString().trim(),'base64'));
    const writeFile=(path,content)=>python('import sys; open(sys.argv[1],"wb").write(sys.stdin.buffer.read())',[path],Buffer.from(content));
    const access=path=>python('import os,sys; assert os.access(sys.argv[1],os.R_OK), "File is not readable"',[path]);
    // Reuse Pi's signature detector (including APNG exclusion), not extensions
    // or a second implementation. Both sniffing and reading run in the Branch.
    const detectImageMimeType=async path=>{
        const quote=value=>"'"+String(value).replaceAll("'","'\\''")+"'";
        const module=new URL('../../../pi/packages/coding-agent/src/utils/mime.ts',import.meta.url).href;
        const code=`import {detectSupportedImageMimeTypeFromFile as detect} from ${JSON.stringify(module)}; console.log(await detect(process.argv[1])??'')`;
        const chunks=[];
        const result=await exec('node --input-type=module -e '+quote(code)+' -- '+quote(path),cwd,{onData:data=>chunks.push(data)});
        const output=Buffer.concat(chunks).toString().trim();
        if(result.exitCode!==0)throw Error(output);
        return output||null;
    };
    const options={bash:{operations:{exec}},read:{operations:{readFile,access,detectImageMimeType}},edit:{operations:{readFile,writeFile,access}},write:{operations:{writeFile,mkdir:path=>python('import os,sys; os.makedirs(sys.argv[1],exist_ok=True)',[path])}}};
    return ['read','bash','edit','write'].map(name=>{
        const tool=sdk.createToolDefinition(name,cwd,options),execute=tool.execute;
        return {...tool,execute:(id,args,signal,...rest)=>context.run(signal,()=>execute(id,args,signal,...rest))};
    });
}
