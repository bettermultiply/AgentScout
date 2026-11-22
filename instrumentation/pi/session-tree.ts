// @ts-nocheck
import { writeFileSync, renameSync, openSync, fsyncSync, closeSync, existsSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { checksum } from './recovery.ts';

export class SessionTree {
    constructor(manager, authority='authority') {
        this.manager=manager;
        this.authority=authority;
        this.tails=new Map();
        this.branches=new Map();
        this.inputs=new Map();
        this.acceptanceParents=new Map();
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

    // Keep the full native index: custom and summary entries can refer off-path.
    createBranch(branchId,workspace,source=this.manager,leaf=source.getLeafId(),persist=true) {
        const identity=source.constructor.create(workspace);
        const header={...identity.getHeader(),parentSession:source.getSessionFile()};
        const manager=source.constructor.inMemory(workspace,undefined,[header,...source.getEntries()]);
        // Pi's preloaded memory sessions need a path before they can persist.
        manager.sessionFile=identity.getSessionFile();
        if(leaf)manager.branch(leaf);else manager.resetLeaf();
        manager.appendCustomEntry('authority',{branch_id:branchId,parentId:leaf??null});
        if(persist)SessionTree.persist(manager);
        if(leaf)manager.branch(leaf);else manager.resetLeaf();
        this.branches.set(branchId,{manager,stopped:false});
        this.tails.set(branchId,leaf);
        return manager;
    }

    static persist(manager) {
        if(manager.isPersisted()&&existsSync(manager.getSessionFile()))return;
        SessionTree.write(manager.getSessionFile(),manager.getHeader(),manager.getEntries());
        manager.persist=true;
        manager.flushed=true;
    }

    static write(file,header,entries) {
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

    accept(branchId, parent, global) {
        this.accepted ??= new Set();
        let inherited = branchId;
        const entries = this.branches.get(branchId)?.manager.getBranch() ?? [];
        do {
            this.accepted.add(inherited);
            const input = this.inputs.get(inherited);
            const first = input && entries.find(entry => entry.parentId === input.pairedTail);
            if (first) input.first = first.id;
            inherited = this.acceptanceParents.get(inherited);
        } while (inherited);
        if (global) this.promote(branchId);
        else this.acceptanceParents.set(branchId, parent);
    }

    adoptAuthority(branchId) { this.authority=branchId; }

    context(branchId) {
        const manager=this.branches.get(branchId)?.manager??this.manager,tail=this.tail(branchId);
        if(tail)manager.branch(tail);else manager.resetLeaf();
        return manager.buildSessionContext();
    }
    assertCheckpoint(branchId,saved) {
        const manager=this.branches.get(branchId)?.manager??this.manager;
        const last=saved.at(-1), recorded=manager.buildSessionContext().messages.at(-1);
        if(JSON.stringify(recorded)!==JSON.stringify(last))throw Error(`Checkpoint tree tail is not the saved final message for branch ${branchId}`);
        return manager.getLeafId();
    }

    stop(branchId) { const branch=this.branches.get(branchId);if(branch)branch.stopped=true; }

    snapshot(directory, checkpointManagers=[]) {
        const managers=new Map([[this.manager.getSessionFile(),this.manager]]);
        for(const {manager} of this.branches.values())managers.set(manager.getSessionFile(),manager);
        for(const input of this.inputs.values())if(input.source)managers.set(input.source.getSessionFile(),input.source);
        for(const manager of checkpointManagers)managers.set(manager.getSessionFile(),manager);
        const copies=new Map();
        if(directory){
            for(const [file,manager] of managers){
                SessionTree.persist(manager);
                if(!existsSync(file))throw Error(`Pi session is not persisted: ${file}`);
                const saved=manager.constructor.forkFrom(file,manager.getHeader().cwd);
                const target=saved.getSessionFile();
                const descriptor=openSync(target,'r');try{fsyncSync(descriptor)}finally{closeSync(descriptor)}
                copies.set(file,target);
            }
        }
        const sourceFile=file=>copies.get(file)??file;
        const sources=[...managers].map(([file,manager])=>({file:sourceFile(file),originalFile:file,leaf:manager.getLeafId(),
            header:manager.constructor.open(sourceFile(file)).getHeader(),entries:structuredClone(manager.getEntries()),
            checksum:checksum(readFileSync(sourceFile(file)))}));
        return {authority:this.authority,manager:sourceFile(this.manager.getSessionFile()),sources,
            branches:[...this.branches].map(([id,{manager,stopped}])=>[id,{file:sourceFile(manager.getSessionFile()),stopped}]),
            inputs:[...this.inputs].map(([id,input])=>[id,{...input,source:input.source?sourceFile(input.source.getSessionFile()):undefined}]),
            tails:[...this.tails],originalEntries:[...this.originalEntries],accepted:[...(this.accepted??[])],
            acceptanceParents:[...this.acceptanceParents]};
    }

    static restore(record, SessionManager, sessionFile) {
        const managers=new Map();
        for(const source of record.sources){
            if(checksum(readFileSync(source.file))!==source.checksum)throw Error(`Recovery source changed: ${source.file}`);
            const records=readFileSync(source.file,'utf8').trim().split('\n').map(JSON.parse);
            const ids=new Set(source.entries.map(e=>e.id));
            const entries=new Map(source.entries.map(e=>[e.id,e]));
            if(ids.size!==source.entries.length||source.leaf&&!ids.has(source.leaf))throw Error('Invalid recovery Pi index');
            if(records.length!==source.entries.length+1)throw Error('Recovery Pi index length mismatch');
            if(JSON.stringify(records[0])!==JSON.stringify(source.header))throw Error('Recovery Pi header mismatch');
            for(const entry of source.entries){
                if(!records.some(e=>e.id===entry.id&&JSON.stringify(e)===JSON.stringify(entry)))throw Error(`Recovery entry missing: ${entry.id}`);
                if(entry.parentId&&!ids.has(entry.parentId))throw Error(`Recovery parent missing: ${entry.parentId}`);
                for(const field of ['targetId','firstKeptEntryId','fromId'])if(entry[field]&&entry[field]!=='root'&&!ids.has(entry[field]))throw Error(`Recovery ${field} missing: ${entry[field]}`);
                const seen=new Set();let current=entry;
                while(current){
                    if(seen.has(current.id))throw Error('Cyclic recovery Pi index');
                    seen.add(current.id);current=entries.get(current.parentId);
                }
            }
        }
        if(!record.sources.some(s=>s.file===record.manager))throw Error('Recovery assembly source missing');
        const sources=new Map(record.sources.map(s=>[s.file,s]));
        if(sources.size!==record.sources.length)throw Error('Duplicate recovery Pi source');
        const branches=new Map(record.branches),tails=new Map(record.tails);
        if(branches.size!==record.branches.length||tails.size!==record.tails.length)throw Error('Duplicate recovery branch or tail');
        if(!branches.has(record.authority))throw Error('Recovery authority source missing');
        for(const [id,branch] of branches){
            const source=sources.get(branch.file);
            if(!source)throw Error('Recovery branch source missing');
            const tail=tails.get(id);
            if(tail&&!source.entries.some(e=>e.id===tail))throw Error('Recovery branch tail missing');
        }
        for(const [id,input] of record.inputs){
            if((!branches.has(input.parent)&&!input.source)||input.source&&!sources.has(input.source))throw Error('Recovery prediction source missing');
        }
        for(const source of record.sources){
            const manager=SessionManager.forkFrom(source.file,source.header.cwd);
            if(source.leaf)manager.branch(source.leaf);else manager.resetLeaf();
            managers.set(source.file,manager);
        }
        const tree=new SessionTree(managers.get(record.manager),record.authority);
        tree.authority=record.authority;tree.branches=new Map(record.branches.map(([id,b])=>{
            const manager=managers.get(b.file);if(!manager)throw Error('Recovery branch source missing');
            return [id,{manager,stopped:b.stopped}];
        }));
        tree.inputs=new Map(record.inputs.map(([id,input])=>{
            const source=input.source?managers.get(input.source):undefined;
            if(input.source&&!source)throw Error('Recovery prediction source missing');
            return [id,{...input,source}];
        }));
        tree.tails=new Map(record.tails);
        tree.originalEntries=new Map(record.originalEntries);tree.accepted=new Set(record.accepted);
        tree.acceptanceParents=new Map(record.acceptanceParents ?? []);
        // Checkpoints can outlive their source Branch.
        tree.recoveryManagers=managers;
        return tree;
    }

    // Assembly may read live managers synchronously but must leave their context intact.
    settle() {
        if(this.readOnlyUntilPrompt)return;
        if(this.branches.size===0){
            const tail=this.tail(this.authority);if(!tail)return;
            const last=this.manager.getEntries().at(-1);
            if(last?.customType==='authority'&&last.data?.branch_id===this.authority&&last.data?.parentId===tail)return;
            this.manager.branch(tail);this.manager.appendCustomEntry('authority',{branch_id:this.authority,parentId:tail});return;
        }
        for(const input of this.inputs.values())if(input.source&&input.parent===this.authority)SessionTree.persist(input.source);
        const entries=new Map(this.originalEntries);
        for(const [branchId,{manager}] of this.branches){
            if(branchId===this.authority||this.accepted?.has(branchId))SessionTree.persist(manager);
            this.tails.set(branchId,manager.getLeafId());
            for(const source of manager.getEntries()){
                const previous=entries.get(source.id);
                if(previous&&previous!==source&&JSON.stringify(previous)!==JSON.stringify(source))throw Error(`Conflicting native Pi entry ID ${source.id}`);
                entries.set(source.id,source);
            }
        }
        for(const [id,entry] of entries){
            if(entry.parentId&&!entries.has(entry.parentId))throw Error(`Missing Pi parent ${entry.parentId} for ${id}`);
            for(const field of ['targetId','firstKeptEntryId','fromId'])if(entry[field]&&entry[field]!=='root'&&!entries.has(entry[field]))throw Error(`Missing Pi ${field} ${entry[field]} for ${id}`);
            if(entry.type==='custom'&&entry.customType==='rollback-snapshot'&&entry.data?.targetId&&!entries.has(entry.data.targetId))throw Error(`Missing rollback target ${entry.data.targetId} for ${id}`);
        }
        this.originalEntries=new Map(entries);
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
            // The copied index may also contain predictions and unrelated paths.
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
            entries.set(first.id,{...entries.get(first.id),parentId:input.realTail});
        }
        for(const entry of entries.values()){
            const path=new Set();let current=entry;
            while(current){
                if(path.has(current.id))throw Error(`Cyclic Pi history at ${current.id}`);
                path.add(current.id);current=entries.get(current.parentId);
            }
        }
        const tail=this.tail(this.authority),marker={type:'custom',customType:'authority',id:randomUUID(),parentId:tail,timestamp:new Date().toISOString(),data:{branch_id:this.authority,parentId:tail}};
        SessionTree.write(this.manager.getSessionFile(),this.manager.getHeader(),[...entries.values(),marker]);
        this.manager=this.manager.constructor.open(this.manager.getSessionFile());
        for(const [branchId,branch] of this.branches)if(branch.stopped){
            // Preserve native history files: checkpoints can reference a released Branch.
            this.branches.delete(branchId);
        }
    }
}
