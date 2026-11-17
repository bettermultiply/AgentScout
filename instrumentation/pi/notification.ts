// @ts-nocheck
import { readFileSync } from 'node:fs';

/** Delivery is confirmed from native persisted entries, never queue acceptance. */
export class Notifications {
    constructor() { this.pending=new Map(); }
    entry(manager,id) {
        return manager.getEntries().find(e=>e.type==='custom_message'&&e.customType==='notification'&&e.details?.command_id===id);
    }
    async accept(session,command) {
        if(command.notification===undefined)return;
        if(typeof command.notification!=='string'||!command.notification.trim()||!command.command_id)throw Error('Invalid notification');
        if(this.pending.has(command.command_id)||this.entry(session.sessionManager,command.command_id))return;
        this.pending.set(command.command_id,{session,command});
        await this.send(session,command);
    }
    async send(session,command) {
        await session.sendCustomMessage({customType:'notification',content:command.notification,display:false,details:{command_id:command.command_id}},{triggerTurn:false});
    }
    confirm(manager) {
        if(![...this.pending.values()].some(item=>item.session.sessionManager===manager))return;
        const records=readFileSync(manager.getSessionFile(),'utf8').split('\n').filter(Boolean).map(JSON.parse);
        for(const [id,item] of this.pending){
            if(item.session.sessionManager!==manager)continue;
            const entry=this.entry(manager,id);
            if(entry){
                if(!records.some(e=>e.id===entry.id&&JSON.stringify(e)===JSON.stringify(entry)))throw Error(`Notification persistence unconfirmed: ${id}`);
                this.pending.delete(id);
            }
        }
    }
    async drain(session) {
        if(![...this.pending.values()].some(item=>item.session===session))return;
        for(const [id,{session:owner,command}] of this.pending){
            if(owner!==session)continue;
            if(!this.entry(session.sessionManager,id))await this.send(session,command);
        }
        this.confirm(session.sessionManager);
        const missing=[...this.pending].filter(([,item])=>item.session===session).map(([id])=>id);
        if(missing.length)throw Error(`Notification persistence unconfirmed: ${missing.join(', ')}`);
    }
}
