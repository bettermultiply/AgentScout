// @ts-nocheck
import { mkdirSync, writeFileSync, renameSync, unlinkSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { randomUUID } from 'node:crypto';

export class SessionTree {
    constructor(manager, authority='authority') {
        this.manager=manager;
        this.authority=authority;
        this.tails=new Map();
        this.branches=new Map();
        this.inputs=new Map();
        this.requests=new Map(manager.getEntries().filter(e=>e.customType==='authority'&&e.data?.requests).flatMap(e=>Object.entries(e.data.requests)));
        this.originalEntries=new Map(manager.getEntries().filter(e=>!(e.type==='custom'&&e.customType==='authority'&&e.data?.branch_id)).map(e=>[e.id,structuredClone(e)]));
        const entries=manager.getEntries(),last=entries.at(-1),marker=[...entries].reverse().find(entry=>entry.type==='custom'&&entry.customType==='authority'&&entry.data?.branch_id&&entry.data?.parentId);
        if(marker){this.authority=marker.data.branch_id;this.tails.set(this.authority,marker.data.parentId)}
        else if(last)this.tails.set(authority,last.id);
    }

    initialize(model,thinkingLevel) {
        if(this.manager.getEntries().length!==0)return;
        if(model)this.manager.appendModelChange(model.provider,model.id);
        this.manager.appendThinkingLevelChange(thinkingLevel);
        this.tails.set(this.authority,this.manager.getLeafId());
    }

    // Copy native records, never reconstruct them from model-visible messages.
    // Keep the full source index: custom/summary entries can refer off-path.
    createBranch(branchId,workspace,source=this.manager,leaf=source.getLeafId()) {
        const directory=join(dirname(this.manager.getSessionFile()),'sessions');
        mkdirSync(directory,{recursive:true});
        const file=join(directory,`${randomUUID()}.jsonl`);
        const header={...source.getHeader(),id:randomUUID(),timestamp:new Date().toISOString(),cwd:workspace,parentSession:source.getSessionFile()};
        const entries=structuredClone(source.getEntries());
        // Native createBranchedSession mutates its receiver and recreates label
        // IDs. Copy native records instead, preserving IDs and off-path custom
        // references, and explicitly select the checkpoint on the new manager.
        this.write(file,header,entries);
        const manager=source.constructor.open(file,undefined,workspace);
        if(leaf)manager.branch(leaf);else manager.resetLeaf();
        // branch() only selects an in-memory leaf. A neutral native record
        // persists the checkpoint path for an independent open of this file.
        manager.appendCustomEntry('authority',{branch_id:branchId,parentId:leaf??null});
        if(leaf)manager.branch(leaf);else manager.resetLeaf();
        this.branches.set(branchId,{manager,stopped:false});
        this.tails.set(branchId,leaf);
        return manager;
    }

    write(file,header,entries) {
        const temporary=`${file}.${randomUUID()}.tmp`;
        writeFileSync(temporary,[header,...entries].map(e=>JSON.stringify(e)).join('\n')+'\n',{flag:'wx'});
        renameSync(temporary,file);
    }

    append(branchId,message) {
        const manager=this.branches.get(branchId)?.manager??this.manager;
        const tail=this.tails.get(branchId);
        if(tail)manager.branch(tail);else manager.resetLeaf();
        const id=manager.appendMessage(message);this.tails.set(branchId,id);return id;
    }
    fork(childBranchId,fromEntryId) {
        if(!this.manager.getEntry(fromEntryId))throw Error(`Entry ${fromEntryId} not found`);
        this.tails.set(childBranchId,fromEntryId);
    }
    tail(branchId) { return this.tails.get(branchId); }
    promote(branchId) {
        if(!this.tails.has(branchId))throw Error(`Branch ${branchId} has no tree tail`);
        this.authority=branchId;
    }


    context(branchId) {
        const manager=this.branches.get(branchId)?.manager??this.manager,tail=this.tail(branchId);
        if(tail)manager.branch(tail);else manager.resetLeaf();
        return manager.buildSessionContext();
    }
    assertCheckpoint(branchId,saved) {
        const manager=this.branches.get(branchId)?.manager??this.manager;
        const last=saved.at(-1),entry=[...manager.getBranch()].reverse().find(e=>e.type==='message');
        if(!entry||JSON.stringify(entry.message)!==JSON.stringify(last))throw Error(`Checkpoint tree tail is not the saved final message for branch ${branchId}`);
        if(last?.role!=='assistant'||!last.content?.some(p=>p.type==='toolCall'))throw Error(`Checkpoint final message is not a pending assistant tool call for branch ${branchId}`);
        return manager.getLeafId();
    }

    stop(branchId) { const branch=this.branches.get(branchId);if(branch)branch.stopped=true; }

    // Read native indexes at a synchronous boundary. Active managers are never
    // rewritten, reloaded, disposed or deleted by history assembly.
    settle() {
        if(this.branches.size===0){
            const tail=this.tail(this.authority);if(!tail)return;
            const last=this.manager.getEntries().at(-1);
            if(last?.customType==='authority'&&last.data?.branch_id===this.authority&&last.data?.parentId===tail)return;
            this.manager.branch(tail);this.manager.appendCustomEntry('authority',{branch_id:this.authority,parentId:tail});return;
        }
        const entries=new Map([...this.originalEntries].map(([id,e])=>[id,structuredClone(e)]));
        for(const [branchId,{manager}] of this.branches){
            this.tails.set(branchId,manager.getLeafId());
            for(const source of manager.getEntries()){
                const previous=entries.get(source.id);
                if(previous&&JSON.stringify(previous)!==JSON.stringify(source))throw Error(`Conflicting native Pi entry ID ${source.id}`);
                entries.set(source.id,structuredClone(source));
            }
        }
        for(const [id,entry] of entries){
            if(entry.parentId&&!entries.has(entry.parentId))throw Error(`Missing Pi parent ${entry.parentId} for ${id}`);
            for(const field of ['targetId','firstKeptEntryId','fromId'])if(entry[field]&&entry[field]!=='root'&&!entries.has(entry[field]))throw Error(`Missing Pi ${field} ${entry[field]} for ${id}`);
            if(entry.type==='custom'&&entry.customType==='rollback-snapshot'&&entry.data?.targetId&&!entries.has(entry.data.targetId))throw Error(`Missing rollback target ${entry.data.targetId} for ${id}`);
        }
        this.originalEntries=new Map([...entries].map(([id,e])=>[id,structuredClone(e)]));
        for(const branchId of this.accepted??[]){
            const input=this.inputs.get(branchId);if(!input)continue;
            const source=input.source??this.branches.get(input.parent)?.manager;
            const path=(manager,tail)=>{
                const result=[],seen=new Set();
                for(let entry=manager?manager.getEntry(tail):this.originalEntries.get(tail);entry;entry=manager?manager.getEntry(entry.parentId):this.originalEntries.get(entry.parentId)){
                    if(seen.has(entry.id))throw Error(`Cyclic native Pi history at ${entry.id}`);
                    seen.add(entry.id);result.unshift(entry);
                }
                return result;
            };
            // A copied index contains predictions and unrelated paths. Only
            // the source continuation of this exact checkpoint supplies facts.
            let checkpoint=input.checkpoint;
            if(!checkpoint){
                let entry=this.originalEntries.get(input.pairedTail);
                for(let i=0;i<input.operations.length;i++)entry=this.originalEntries.get(entry?.parentId);
                checkpoint=entry?.id;
            }
            if(!input.realTail&&source){
                const sourcePath=path(source,source.getLeafId()),start=sourcePath.findIndex(e=>e.id===checkpoint);
                if(start>=0){
                    const found=new Set();
                    for(const entry of sourcePath.slice(start+1)){
                        if(entry.type!=='message')continue;
                        if(entry.message.role==='assistant')break;
                        if(entry.message.role==='toolResult'&&input.operations.includes(entry.message.toolCallId)){
                            if(found.has(entry.message.toolCallId))throw Error(`Duplicate result after Pi checkpoint ${checkpoint}`);
                            found.add(entry.message.toolCallId);
                            if(found.size===input.operations.length){input.realTail=entry.id;break}
                        }
                    }
                }
            }
            const candidate=this.branches.get(branchId)?.manager;
            const candidatePath=path(candidate,candidate?.getLeafId()??this.tail(branchId));
            const first=input.first?entries.get(input.first):candidatePath.find(e=>e.parentId===input.pairedTail);
            if(!first)continue;
            input.first=first.id;
            if(!input.realTail)continue;
            entries.get(first.id).parentId=input.realTail;
        }
        for(const entry of entries.values()){
            const path=new Set();let current=entry;
            while(current){
                if(path.has(current.id))throw Error(`Cyclic Pi history at ${current.id}`);
                path.add(current.id);current=entries.get(current.parentId);
            }
        }
        const tail=this.tail(this.authority),marker={type:'custom',customType:'authority',id:randomUUID(),parentId:tail,timestamp:new Date().toISOString(),data:{branch_id:this.authority,parentId:tail,requests:Object.fromEntries(this.requests)}};
        this.write(this.manager.getSessionFile(),this.manager.getHeader(),[...entries.values(),marker]);
        // Re-open only the assembly manager. Runtime managers keep their own
        // original predicted contexts, including any inference already in flight.
        this.manager=this.manager.constructor.open(this.manager.getSessionFile());
        for(const [branchId,branch] of this.branches)if(branch.stopped){
            // Entries remain in the manager until assembly succeeds. A failed
            // write/validation never removes its native source file.
            unlinkSync(branch.manager.getSessionFile());
            this.branches.delete(branchId);
        }
    }
}
