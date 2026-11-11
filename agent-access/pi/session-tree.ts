// @ts-nocheck

export class SessionTree {
    constructor(manager, authority='authority') {
        this.manager=manager;
        this.authority=authority;
        this.tails=new Map();
        const entries=manager.getEntries(),last=entries.at(-1),marker=[...entries].reverse().find(entry=>entry.type==='custom'&&entry.customType==='authority'&&entry.data?.branch_id&&entry.data?.parentId);
        if(marker){
            this.authority=marker.data.branch_id;
            this.tails.set(this.authority,marker.data.parentId);
        } else if(last) this.tails.set(authority,last.id);
    }

    initialize(model,thinkingLevel) {
        if(this.manager.getEntries().length!==0)return;
        if(model)this.manager.appendModelChange(model.provider,model.id);
        this.manager.appendThinkingLevelChange(thinkingLevel);
        const entries=this.manager.getEntries();
        if(entries.length)this.tails.set(this.authority,entries.at(-1).id);
    }

    append(branchId,message) {
        const tail=this.tails.get(branchId);
        if(tail)this.manager.branch(tail);
        else this.manager.resetLeaf();
        const entryId=this.manager.appendMessage(message);
        this.tails.set(branchId,entryId);
        return entryId;
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
        const tail=this.tails.get(branchId);
        if(tail)this.manager.branch(tail);
        else this.manager.resetLeaf();
        return this.manager.buildSessionContext();
    }

    assertCheckpoint(branchId,saved) {
        const tail=this.tail(branchId);
        const entry=tail&&this.manager.getEntry(tail);
        const last=saved.at(-1);
        if(!entry||entry.type!=='message'||entry.message!==last&&JSON.stringify(entry.message)!==JSON.stringify(last))
            throw Error(`Checkpoint tree tail is not the saved final message for branch ${branchId}`);
        if(last?.role!=='assistant'||!last.content?.some(part=>part.type==='toolCall'))
            throw Error(`Checkpoint final message is not a pending assistant tool call for branch ${branchId}`);
        return tail;
    }

    settle() {
        const tail=this.tail(this.authority);
        if(!tail)return;
        const entries=this.manager.getEntries(),last=entries.at(-1);
        if(last?.type==='custom'&&last.customType==='authority'&&last.data?.branch_id===this.authority&&last.data?.parentId===tail)return;
        this.manager.branch(tail);
        this.manager.appendCustomEntry('authority',{branch_id:this.authority,parentId:tail});
    }
}
