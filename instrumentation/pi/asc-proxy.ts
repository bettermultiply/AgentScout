import {connect} from 'node:net';
import { carrier } from './telemetry.ts';
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
    private sequences=new Map<string,number>();
    private operationSequence=0;
    private generations=new Map<string,number>();
    private cancelling=new Map<string,Promise<void>>();
    bind(branch:string,access:Access|undefined){if(access)this.accesses.set(branch,access)}
    unbind(branch:string){this.accesses.delete(branch)}
    transfer(from:string,to:string){
        const access=this.accesses.get(from);
        if(!access)throw Error('Parent execution access missing');
        this.accesses.set(to,access);
        this.accesses.delete(from);
    }
    has(branch:string){return this.accesses.has(branch)}
    private async request(access: Access, operation: string, command: string, extra: Record<string, unknown> = {}, accepted?: () => void, output?: (value: any) => void): Promise<any> {
        if (access.transport !== 'unix-json-lines') throw Error('Branch execution access missing or unsupported');
        const request = { ...extra, command, operation_id: operation, environment_id: access.environment_id, trace_context: carrier() };
        let parts: Uint8Array[] | undefined;
        if (typeof extra.operation === 'string') {
            const body = operationBody(extra.operation, extra.arguments as Record<string, unknown>);
            Object.assign(request, { arguments: body.metadata });
            parts = body.parts;
        }
        const socket = connect(access.address);
        // Keep socket errors handled even when the iterator is inactive.
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
        const access = this.accesses.get(branch);
        if (!access) throw Error('Branch execution access missing');
        const environment = access.environment_id;
        if(this.cancelling.has(environment))throw Error('Environment cancellation in progress');
        const identity=JSON.stringify([environment,operation]);
        if(this.active.has(identity))throw Error('Operation is already executing');
        let accept!:()=>void,failAcceptance!:(error:unknown)=>void;
        const active={accepted:new Promise<void>((resolve,reject)=>{accept=resolve;failAcceptance=reject}),cancel:undefined as Promise<unknown>|undefined};
        void active.accepted.catch(()=>{});
        this.active.set(identity,active);
        const sequence=(this.sequences.get(environment)??0)+1;
        this.sequences.set(environment,sequence);
        const result=this.request(access,operation,'execute',{request_id:`req-${sequence}`,generation:this.generations.get(environment)??0,...(environmentOperation?{operation:environmentOperation}:{tool}),arguments:args,context},accept,output);
        void result.catch(failAcceptance);
        const abort=()=>{active.cancel??=active.accepted.then(()=>this.request(access,operation,'cancel'));void active.cancel.catch(()=>{})};
        signal?.addEventListener('abort',abort,{once:true});
        if(signal?.aborted)abort();
        try {
            const {event, body}=await result;
            try { await active.cancel; }
            finally { await this.request(access,operation,'confirm'); }
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
        const access=this.accesses.get(branch);
        if(!access)return;
        const environment=access.environment_id;
        const pending=this.cancelling.get(environment);
        if(pending)return pending;
        const cancellation=this.cancelEnvironment(access);
        this.cancelling.set(environment,cancellation);
        try{await cancellation}finally{this.cancelling.delete(environment)}
    }
    private async cancelEnvironment(access:Access){
        const environment=access.environment_id;
        // Cancellation needs admission first. Environment drain covers interrupted cancels.
        await Promise.allSettled([...this.active].map(async([identity,active])=>{
            const [owner,operation]=JSON.parse(identity);
            if(owner===environment){
                active.cancel??=active.accepted.then(()=>this.request(access,operation,'cancel'));
                await active.cancel;
            }
        }));
        const terminal=await this.request(access,'environment-cancellation','cancel_environment');
        this.generations.set(environment,terminal.event.generation);
    }
}
