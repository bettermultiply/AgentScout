// Pi sessions remain in their host process; every built-in file operation uses
// the plan of its own Branch. A fresh mount per operation permits baseline
// replacement between tasks without touching any live overlay.
import {readFileSync,openSync,closeSync} from 'node:fs';
import {spawn} from 'node:child_process';
import {connect as connectSocket} from 'node:net';
import {AsyncLocalStorage} from 'node:async_hooks';
import {join,dirname,basename} from 'node:path';

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

type ToolResult = {exitCode: number | undefined};
type ToolOptions = {
    onData?: (chunk: Buffer) => void;
    signal?: AbortSignal;
    timeout?: number;
    env?: Record<string, string>;
    input?: Buffer | string;
};

export function filesystemTools(directory: string, branch: string, cwd: string, sdk: any) {
    if(!directory)return [];
    const context=new AsyncLocalStorage<AbortSignal | undefined>();
    // Per-operation namespaces must finish (including adopted descendants)
    // before the next operation sees their writes. Persistent holders instead
    // own background work for the environment and deliberately allow concurrency.
    let pending: Promise<unknown>=Promise.resolve();
    const planFile=join(directory,branch+'.json');
    const readPlan=()=>JSON.parse(readFileSync(planFile,'utf8'));
    // An isolated host directory can be mounted at another path inside the
    // branch namespace (for example a Terminal-Bench workspace at /app). Pi's
    // session cwd is the host path; tool operations run in the branch and must
    // use the sandbox path. This mapping comes from the branch tool plan, not
    // from a hard-coded workload path.
    const translate=(value: string): string=>{
        let plan;
        try{plan=readPlan()}catch{return value}
        const workspace=plan.workspace,target=workspace?.target,host=workspace?.host;
        if(!plan.holder_socket||!target||!host||host===target)return value;
        if(value===host)return target;
        return value.startsWith(host+'/')?target+value.slice(host.length):value;
    };
    const exec=(command: string, cwd: string, options: ToolOptions = {}): Promise<ToolResult>=>{
        const signal=options.signal??context.getStore();
        const execute=()=>run(command,translate(cwd),{...options,signal});
        try{if(readPlan().holder_socket)return execute()}catch(error){return Promise.reject(error)}
        const result=pending.then(execute);
        // A failed write must not poison subsequent operations. run still waits
        // for close before rejecting EPIPE, cancellation or timeout.
        pending=result.catch(()=>{});
        return result;
    };
    const run=(command: string, cwd: string, options: ToolOptions = {}): Promise<ToolResult>=>new Promise((resolve,reject)=>{
        let {onData,signal,timeout,env,input}=options;
        signal??=context.getStore();
        if(signal?.aborted){reject(Error('Tool cancelled'));return}
        let plan;
        try{plan=readPlan()}catch(error){reject(error);return}
        if(plan.holder_socket){
            const directoryFd=openSync(dirname(plan.holder_socket),'r');
            let socket: ReturnType<typeof connectSocket>;
            try{socket=connectSocket(`/proc/self/fd/${directoryFd}/${basename(plan.holder_socket)}`)}
            catch(error){closeSync(directoryFd);reject(error);return}
            let finished=false,timedOut=false,buffer='';
            const timer=timeout?setTimeout(()=>{timedOut=true;socket.destroy()},timeout*1000):undefined;
            const stop=()=>socket.destroy();
            const finish=(error?: Error | null, result?: ToolResult)=>{
                if(finished)return;finished=true;
                closeSync(directoryFd);
                if(timer)clearTimeout(timer);
                signal?.removeEventListener('abort',stop);
                error?reject(error):resolve(result as ToolResult);
            };
            signal?.addEventListener('abort',stop,{once:true});
            if(signal?.aborted)stop();
            socket.on('data',chunk=>{
                buffer+=chunk.toString();
                let index;
                while((index=buffer.indexOf('\n'))>=0){
                    const line=buffer.slice(0,index);buffer=buffer.slice(index+1);
                    if(!line)continue;
                    let event;
                    try{event=JSON.parse(line)}catch(error){finish(error instanceof Error?error:Error(String(error)));socket.destroy();return}
                    if(event.stdout)onData?.(Buffer.from(event.stdout,'base64'));
                    if(event.stderr)onData?.(Buffer.from(event.stderr,'base64'));
                    if(event.error!==undefined){finish(Error(event.error));socket.destroy();return}
                    if(event.exit_code!==undefined){finish(null,{exitCode:event.exit_code});socket.destroy();return}
                }
            });
            socket.once('error',error=>finish(error));
            socket.once('close',()=>finish(timedOut?Error('Tool timed out'):signal?.aborted?Error('Tool cancelled'):Error('Namespace holder closed before returning a result')));
            socket.write(JSON.stringify({
                op:'exec',
                command,
                cwd:translate(cwd),
                env:env??{},
                ...(input!==undefined?{input:Buffer.from(input).toString('base64')}:{}),
                ...(timeout?{timeout}:{}),
            })+'\n');
            return;
        }
        const child=spawn(plan.program,[...plan.args,'--chdir',cwd,'--','python3','-c',supervisor,'/bin/bash','-c',command],{env:env??process.env,stdio:[input===undefined?'ignore':'pipe','pipe','pipe'],detached:true});
        // Contents travel only through the pipe into the isolated process, never
        // argv or a host temporary file. Wait for close even if the reader exits
        // early: the namespace supervisor must finish reaping descendants first.
        let inputError: Error | undefined;
        if(child.stdin){child.stdin.on('error',error=>{inputError=error});child.stdin.end(input)}
        child.stdout?.on('data',data=>onData?.(data));child.stderr?.on('data',data=>onData?.(data));
        let timedOut=false;
        const stop=()=>{try{if(child.pid!==undefined)process.kill(-child.pid,'SIGKILL')}catch(error){if((error as NodeJS.ErrnoException).code!=='ESRCH')throw error}};
        const timer=timeout?setTimeout(()=>{timedOut=true;stop()},timeout*1000):undefined;
        signal?.addEventListener('abort',stop,{once:true});
        // Abort may arrive between the pre-spawn check and listener setup.
        if(signal?.aborted)stop();
        child.once('error',reject);
        child.once('close',code=>{clearTimeout(timer);signal?.removeEventListener('abort',stop);if(signal?.aborted||timedOut)reject(Error(timedOut?'Tool timed out':'Tool cancelled'));else if(inputError)reject(inputError);else resolve({exitCode:code??undefined})});
    });
    const python=async(code: string, args: string[], input: Buffer | string | undefined = undefined): Promise<Buffer>=>{
        const quote=(value: unknown)=>"'"+String(value).replaceAll("'","'\\''")+"'";
        const chunks: Buffer[]=[];
        const result=await exec('python3 -c '+quote(code)+' '+args.map(quote).join(' '),cwd,{input,onData:data=>chunks.push(data)});
        const output=Buffer.concat(chunks);
        if(result.exitCode!==0)throw Error(output.toString());
        return output;
    };
    const readFile=(path: string)=>python('import sys,base64; print(base64.b64encode(open(sys.argv[1],"rb").read()).decode())',[translate(path)]).then(data=>Buffer.from(data.toString().trim(),'base64'));
    const writeFile=(path: string, content: Buffer | string)=>python('import sys; open(sys.argv[1],"wb").write(sys.stdin.buffer.read())',[translate(path)],Buffer.from(content));
    const access=(path: string)=>python('import os,sys; assert os.access(sys.argv[1],os.R_OK), "File is not readable"',[translate(path)]);
    // Reuse Pi's signature detector (including APNG exclusion), not extensions
    // or a second implementation. Both sniffing and reading run in the Branch.
    const detectImageMimeType=async (path: string): Promise<string | null>=>{
        const quote=(value: unknown)=>"'"+String(value).replaceAll("'","'\\''")+"'";
        const module=new URL('../../../pi/packages/coding-agent/src/utils/mime.ts',import.meta.url).href;
        const code=`import {detectSupportedImageMimeTypeFromFile as detect} from ${JSON.stringify(module)}; console.log(await detect(process.argv[1])??'')`;
        const chunks: Buffer[]=[];
        const result=await exec('node --input-type=module -e '+quote(code)+' -- '+quote(translate(path)),cwd,{onData:data=>chunks.push(data)});
        const output=Buffer.concat(chunks).toString().trim();
        if(result.exitCode!==0)throw Error(output);
        return output||null;
    };
    const options={bash:{operations:{exec}},read:{operations:{readFile,access,detectImageMimeType}},edit:{operations:{readFile,writeFile,access}},write:{operations:{writeFile,mkdir:(path: string)=>python('import os,sys; os.makedirs(sys.argv[1],exist_ok=True)',[translate(path)])}}};
    return ['read','bash','edit','write'].map((name: string)=>{
        const tool=sdk.createToolDefinition(name,cwd,options),execute=tool.execute;
        return {...tool,execute:(id: any,args: any,signal?: AbortSignal,...rest: any[])=>context.run(signal,()=>execute(id,args,signal,...rest))};
    });
}
