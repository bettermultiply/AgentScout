import {StringDecoder} from 'node:string_decoder';
import type {AscProxy} from './asc-proxy.ts';

export function environmentTools(proxy:AscProxy,branch:string,detect:(bytes:Uint8Array)=>string|null,taskId:string){
    const string={type:'string'};
    const object=(properties:Record<string,unknown>,required:string[])=>({type:'object',properties,required});
    const text=(value:string,details:unknown=undefined)=>({content:[{type:'text',text:value}],details,structuredContent:value});
    const operation=(id:string,name:string,args:unknown,signal?:AbortSignal,output?:(value:any)=>void)=>proxy.operation(branch,id,name,args,{task_id:taskId},signal,output);
    return [
        {name:'read',label:'read',description:'Read text or images. offset is a 1-indexed line number; limit is a line count. Text output is limited to 2000 lines or 50 KiB; use offset to continue.',parameters:object({path:string,offset:{type:'integer',minimum:1},limit:{type:'integer',minimum:0}},['path']),
            async execute(id:string,args:{path:string;offset?:number;limit?:number},signal?:AbortSignal){
                const result=await operation(id,'read_file',{path:args.path,line_range:{offset:args.offset??1,limit:Math.min(args.limit??2000,2000)},max_bytes:50*1024,binary_fallback:true},signal);
                const bytes=Buffer.from(result.value.bytes),mime=detect(bytes);
                if(mime)return {content:[{type:'text',text:`Read image file [${mime}]`},{type:'image',mimeType:mime,data:bytes.toString('base64')}],details:undefined,structuredContent:{type:'image',mimeType:mime,data:bytes.toString('base64'),note:`Read image file [${mime}]`}};
                const value=result.value,start=value.actual_range.offset,count=value.actual_range.limit;
                let output=bytes.toString('utf8');
                if(value.truncated&&count===0)output=`[Line ${start} exceeds 50 KiB output limit. Use a byte-range Runtime read or bash to inspect it.]`;
                else if(value.has_more&&count>0)output+=`\n\n[Showing lines ${start}-${start+count-1}. Use offset=${start+count} to continue.]`;
                return text(output,{truncated:value.truncated});
            }},
        {name:'write',label:'write',description:'Write complete UTF-8 content, creating parent directories.',parameters:object({path:string,content:string},['path','content']),
            async execute(id:string,args:{path:string;content:string},signal?:AbortSignal){
                const result=await operation(id,'write_file',{path:args.path,bytes:Buffer.from(args.content),create_parents:true},signal);
                return text(`Successfully wrote to ${args.path}`,result.value);
            }},
        {name:'edit',label:'edit',description:'Apply ordered exact replacements to one file. Each oldText must occur exactly once at its step. No fuzzy matching. All replacements are validated before writing.',parameters:object({path:string,edits:{type:'array',minItems:1,items:object({oldText:string,newText:string},['oldText','newText'])}},['path','edits']),
            prepareArguments(input:any){
                let edits=input.edits;
                if(typeof edits==='string'){try{edits=JSON.parse(edits)}catch{}}
                if(edits&&!Array.isArray(edits))edits=[edits];
                if(typeof input.oldText==='string'&&typeof input.newText==='string')edits=[...(edits??[]),{oldText:input.oldText,newText:input.newText}];
                return {path:input.path,edits};
            },
            async execute(id:string,args:{path:string;edits:{oldText:string;newText:string}[]},signal?:AbortSignal){
                if(!Array.isArray(args.edits)||!args.edits.length)throw Error('edits must contain at least one replacement');
                const result=await operation(id,'replace_text',{path:args.path,replacements:args.edits.map(edit=>({old_text:edit.oldText,new_text:edit.newText,match:'exactly_one'}))},signal);
                return text(`Successfully replaced ${args.edits.length} block(s) in ${args.path}.`,result.value);
            }},
        {name:'bash',label:'bash',description:'Execute bash in the bound environment; stream stdout/stderr. A nonzero exit code is command output, not a transport failure.',parameters:object({command:string,timeout:{type:'number',exclusiveMinimum:0}},['command']),
            async execute(id:string,args:{command:string;timeout?:number},signal?:AbortSignal,onUpdate?:(value:unknown)=>void){
                let output='';const decoders:Record<string,StringDecoder>={stdout:new StringDecoder('utf8'),stderr:new StringDecoder('utf8')};
                const result=await operation(id,'exec',{command:args.command,timeout:args.timeout},signal,event=>{if(event.bytes){output+=(decoders[event.stream]??decoders.stdout).write(Buffer.from(event.bytes));onUpdate?.(text(output));}});
                output+=decoders.stdout.end()+decoders.stderr.end();
                const isError=result.exit_code!==0;
                if(isError)output+=`\n\nCommand exited with code ${result.exit_code}`;
                return {...text(output,{exitCode:result.exit_code}),isError};
            }},
    ];
}
