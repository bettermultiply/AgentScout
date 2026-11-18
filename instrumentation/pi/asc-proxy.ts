/** ASC-Proxy owns the separate control and backend-neutral execution channels. */
import {connect} from 'node:net';
import {executionMessages, operationBody, writeExecution} from './execution-wire.ts';
type Access = {transport:string;address:string;environment_id:string};
export class AscProxy {
    private control?:ReturnType<typeof connect>;
    private outgoing=Promise.resolve();
    error?:unknown;
    async open(path:string){
        const socket=connect({path,allowHalfOpen:true});this.control=socket;
        socket.on('error',error=>{this.error=error;socket.destroy()});
        await new Promise<void>((resolve,reject)=>{socket.once('connect',resolve);socket.once('error',reject)});
    }
    destroy(){this.control?.destroy()}
    async send(wire:unknown){
        const socket=this.control;if(!socket||socket.destroyed)return;
        const bytes=Buffer.from(JSON.stringify(wire)),header=Buffer.alloc(4);header.writeUInt32BE(bytes.length);
        this.outgoing=this.outgoing.then(()=>new Promise<void>((resolve,reject)=>{
            if(socket.destroyed){resolve();return}
            socket.write(Buffer.concat([header,bytes]),error=>error?reject(error):resolve());
        })).catch(error=>{this.error=error;socket.destroy()});
        await this.outgoing;
    }
    async *messages(){
        if(!this.control)throw Error('Control connection not open');
        let buffer=Buffer.alloc(0);
        for await(const chunk of this.control.iterator({destroyOnReturn:false})){
            buffer=Buffer.concat([buffer,chunk]);
            while(buffer.length>=4){
                const length=buffer.readUInt32BE(0);if(length>16777216)throw Error('Protocol frame too large');
                if(buffer.length<length+4)break;
                const wire=JSON.parse(buffer.subarray(4,length+4).toString());buffer=buffer.subarray(length+4);
                yield wire;
            }
        }
        if(buffer.length)throw Error('Truncated control frame');
    }
    async close(){await this.outgoing;const socket=this.control;if(socket&&!socket.destroyed)await new Promise<void>(resolve=>socket.end(resolve))}
    private accesses=new Map<string,Access>();
    private active=new Map<string,{accepted:Promise<void>;cancel?:Promise<unknown>}>();
    private sequence=0;
    private operationSequence=0;
    private generations=new Map<string,number>();
    private cancelling=new Map<string,Promise<void>>();
    bind(branch:string,access:Access|undefined){if(access)this.accesses.set(branch,access)}
    has(branch:string){return this.accesses.has(branch)}
    private async request(branch: string, operation: string, command: string, extra: Record<string, unknown> = {}, accepted?: () => void, output?: (value: any) => void): Promise<any> {
        const access = this.accesses.get(branch);
        if (!access || access.transport !== 'unix-json-lines') throw Error('Branch execution access missing or unsupported');
        const request = { ...extra, command, branch_id: branch, operation_id: operation, environment_id: access.environment_id };
        let parts: Uint8Array[] | undefined;
        if (typeof extra.operation === 'string') {
            const body = operationBody(extra.operation, extra.arguments as Record<string, unknown>);
            Object.assign(request, { arguments: body.metadata });
            parts = body.parts;
        }
        const socket = connect(access.address);
        // The async iterator also receives socket errors. Keep a listener on
        // during setup/teardown, when the iterator may have released its own.
        socket.on('error', () => {});
        try {
            await writeExecution(socket, request, parts);
            for await (const { metadata: event, body } of executionMessages(socket)) {
                switch (event.event) {
                    case 'rejected': throw Error(event.reason);
                    case 'accepted': accepted?.(); break;
                    case 'output':
                        output?.(body === undefined ? event.value : { ...event.value, bytes: body });
                        break;
                    case 'terminal':
                    case 'confirmed':
                        return { event, body };
                    default: throw Error('Unknown execution event');
                }
            }
            throw Error('Execution connection closed before terminal confirmation');
        } finally {
            socket.destroy();
        }
    }
    async execute(branch:string,operation:string,tool:string,args:unknown,context:unknown,signal?:AbortSignal,output?:(value:any)=>void,environmentOperation?:string){
        if(signal?.aborted)throw Error('Tool cancelled before execution');
        if(this.cancelling.has(branch))throw Error('Branch cancellation in progress');
        const identity=JSON.stringify([branch,operation]);
        if(this.active.has(identity))throw Error('Operation is already executing');
        let accept!:()=>void,failAcceptance!:(error:unknown)=>void;
        const active={accepted:new Promise<void>((resolve,reject)=>{accept=resolve;failAcceptance=reject}),cancel:undefined as Promise<unknown>|undefined};
        void active.accepted.catch(()=>{});
        this.active.set(identity,active);
        const result=this.request(branch,operation,'execute',{request_id:`req-${++this.sequence}`,generation:this.generations.get(branch)??0,...(environmentOperation?{operation:environmentOperation}:{tool}),arguments:args,context},accept,output);
        void result.catch(failAcceptance);
        const abort=()=>{active.cancel??=active.accepted.then(()=>this.request(branch,operation,'cancel'));void active.cancel.catch(()=>{})};
        signal?.addEventListener('abort',abort,{once:true});
        if(signal?.aborted)abort();
        try {
            const {event, body}=await result;
            try { await active.cancel; }
            finally { await this.request(branch,operation,'confirm'); }
            if(event.status!=='succeeded')throw Error(event.reason??event.status);
            if (environmentOperation) {
                const {value, exit_code} = event;
                if (event.body_kind === 'file_bytes' && body !== undefined) return {value: {...value, bytes: body}};
                if (event.body_kind === 'json_value' && body !== undefined) return {value: JSON.parse(body.toString('utf8'))};
                if (event.body_kind !== undefined || body !== undefined) throw Error('Invalid environment result body');
                if (exit_code !== undefined) return {exit_code};
                return {value};
            }
            if (event.body_kind !== 'business_output' || !body || !Number.isSafeInteger(event.stdout_length)
                || event.stdout_length < 0 || event.stdout_length > body.length) {
                throw Error('Invalid Runtime business output');
            }
            const value = JSON.parse(body.subarray(0, event.stdout_length).toString('utf8'));
            if(value?.result===undefined)throw Error('Runtime result missing');
            return value;
        } finally {signal?.removeEventListener('abort',abort);this.active.delete(identity)}
    }
    operation(branch:string,toolCall:string,operation:string,args:unknown,context:unknown,signal?:AbortSignal,output?:(value:any)=>void){
        const identity=JSON.stringify([toolCall,++this.operationSequence]);
        return this.execute(branch,identity,'',args,{...(context as object),tool_call_id:toolCall},signal,output,operation);
    }
    async cancel(branch:string){
        const pending=this.cancelling.get(branch);if(pending)return pending;
        const cancellation=this.cancelBranch(branch);this.cancelling.set(branch,cancellation);
        try{await cancellation}finally{this.cancelling.delete(branch)}
    }
    private async cancelBranch(branch:string){
        // A rejected execution has no backend entry to cancel. The branch
        // drain must still run, including when an earlier execute failed
        // admission or its cancellation connection failed.
        await Promise.allSettled([...this.active].map(async([identity,active])=>{const [owner,operation]=JSON.parse(identity);if(owner===branch){active.cancel??=active.accepted.then(()=>this.request(branch,operation,'cancel'));await active.cancel}}));
        if(this.has(branch)){const terminal=await this.request(branch,'branch-cancellation','cancel_branch');this.generations.set(branch,terminal.event.generation)}
    }
}
