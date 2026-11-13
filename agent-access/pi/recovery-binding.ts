// @ts-nocheck
import {existsSync,readFileSync,readdirSync,lstatSync,realpathSync} from 'node:fs';
import {resolve,dirname,join,isAbsolute} from 'node:path';
import {homedir} from 'node:os';
import {execFileSync} from 'node:child_process';
import {checksum} from './recovery.ts';

/** Bind auto-discovered configuration before Pi can load extensions on resume.
 * This is not an extension deployment system: package-managed extensions must
 * provide a supported local resource, otherwise saving fails explicitly.
 */
export function recoveryBinding(piRoot,workspaces) {
    const resources=new Map();
    function fingerprint(path,required=false) {
        path=resolve(path);
        if(resources.has(path))return;
        if(!existsSync(path)){if(required)throw Error(`Recovery resource missing: ${path}`);resources.set(path,null);return}
        const stat=lstatSync(path);
        if(stat.isSymbolicLink()){
            resources.set(path,{realpath:realpathSync(path)});fingerprint(realpathSync(path),true);return;
        }
        if(stat.isFile()){resources.set(path,checksum(readFileSync(path)));return}
        if(!stat.isDirectory())throw Error(`Unsupported recovery resource: ${path}`);
        const names=readdirSync(path).filter(n=>!['.git','node_modules','sessions'].includes(n)).sort();
        resources.set(path,names);for(const name of names)fingerprint(join(path,name),true);
    }
    function settings(path,base) {
        fingerprint(path);
        if(!existsSync(path))return;
        const config=JSON.parse(readFileSync(path,'utf8'));
        if(config.packages?.length)throw Error(`Recovery does not yet support package-managed extensions: ${path}; retain local extension resources instead`);
        for(const resource of [...(config.extensions??[]),...(config.skills??[]),...(config.prompts??[])]) {
            if(typeof resource!=='string')throw Error(`Unsupported recovery resource declaration: ${path}`);
            const name=resource.startsWith('!')?resource.slice(1):resource;
            if(name.includes('*'))throw Error(`Recovery requires explicit local resource paths: ${name}`);
            fingerprint(name.startsWith('~/')?join(homedir(),name.slice(2)):isAbsolute(name)?name:resolve(base,name),true);
        }
    }
    const agentDir=resolve(process.env.PI_CODING_AGENT_DIR??join(homedir(),'.pi/agent'));
    for(const name of ['models.json','extensions','skills','prompts','SYSTEM.md','APPEND_SYSTEM.md'])fingerprint(join(agentDir,name));
    settings(join(agentDir,'settings.json'),agentDir);
    for(const cwd of new Set(workspaces)) {
        let current=resolve(cwd);
        for(;;){
            const local=join(current,'.pi');
            for(const name of ['extensions','skills','prompts','SYSTEM.md','APPEND_SYSTEM.md'])fingerprint(join(local,name));
            settings(join(local,'settings.json'),local);
            fingerprint(join(current,'AGENTS.md'));
            const parent=dirname(current);if(parent===current)break;current=parent;
        }
    }
    for(const name of ['package.json','packages/coding-agent/src','packages/agent/src','packages/ai/src','packages/ascout-agent-access/src'])fingerprint(join(piRoot,name),true);
    fingerprint(new URL('../core',import.meta.url).pathname,true);
    fingerprint(dirname(new URL(import.meta.url).pathname),true);
    return {node:process.version,resources:[...resources].sort(([a],[b])=>a.localeCompare(b))};
}

export function snapshotResources(sources,expected) {
    const records=[];
    for(const source of sources)for(const entry of source.entries) {
        if(entry.type!=='custom'||entry.customType!=='rollback-snapshot')continue;
        const data=entry.data;
        if(!data?.repoRoot||!/^refs\/pi\/rollback\//.test(data.ref??'')||!/^[a-f0-9]{40,64}$/.test(data.tree??''))throw Error('Unsupported rollback snapshot resource');
        const git=(...args)=>execFileSync('git',['-C',data.repoRoot,...args],{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
        const gitDirectory=realpathSync(git('rev-parse','--absolute-git-dir'));
        const commit=git('rev-parse','--verify',`${data.ref}^{commit}`);
        if(git('rev-parse','--verify',`${data.ref}^{tree}`)!==data.tree)throw Error(`Rollback snapshot tree changed: ${data.ref}`);
        git('cat-file','-e',`${data.tree}^{tree}`);
        // Keep objects reachable independently of an extension's session-start GC.
        // No checkout/reset is performed and an existing ref is never overwritten.
        const retainedRef=`refs/ascout/recovery/${checksum(gitDirectory+'\0'+data.ref+'\0'+commit)}`;
        if(expected) {
            if(git('rev-parse','--verify',retainedRef)!==commit)throw Error(`Retained rollback snapshot changed: ${retainedRef}`);
        } else {
            let existing;try{existing=git('rev-parse','--verify',retainedRef)}catch{}
            if(existing&&existing!==commit)throw Error(`Retained rollback snapshot changed: ${retainedRef}`);
            if(!existing)git('update-ref',retainedRef,commit,'0'.repeat(commit.length));
        }
        records.push({repoRoot:realpathSync(data.repoRoot),gitDirectory,ref:data.ref,tree:data.tree,commit,retainedRef});
    }
    const unique=[...new Map(records.map(r=>[r.gitDirectory+'\0'+r.ref,r])).values()].sort((a,b)=>(a.gitDirectory+a.ref).localeCompare(b.gitDirectory+b.ref));
    if(expected&&JSON.stringify(unique)!==JSON.stringify(expected))throw Error('Recovery external Git snapshot binding changed');
    return unique;
}
