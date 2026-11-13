// @ts-nocheck
import {existsSync,readFileSync,readdirSync,lstatSync,realpathSync} from 'node:fs';
import {resolve,dirname,join} from 'node:path';
import {homedir} from 'node:os';
import {execFileSync} from 'node:child_process';
import {checksum} from './recovery.ts';
import {createRequire} from 'node:module';

/** Bind Pi-resolved resources before extension execution on resume. */
export async function recoveryBinding(piRoot,workspaces) {
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
        JSON.parse(readFileSync(path,'utf8'));
        // Declaration syntax and package filters belong to Pi's resolver.
    }
    const agentDir=resolve(process.env.PI_CODING_AGENT_DIR??join(homedir(),'.pi/agent'));
    const require=createRequire(resolve(piRoot,'package.json'));
    const {DefaultPackageManager,SettingsManager}=require(resolve(piRoot,'packages/coding-agent/src/index.ts'));
    const resolved=[];
    for(const cwd of [...new Set(workspaces)].sort()) {
        const manager=new DefaultPackageManager({cwd,agentDir,settingsManager:SettingsManager.create(cwd,agentDir)});
        // Recovery never installs or runs extension code during validation.
        const paths=await manager.resolve(async()=> 'error');
        if(paths.installFailures?.length)throw Error(`Recovery package resolution failed: ${JSON.stringify(paths.installFailures)}`);
        resolved.push({cwd,paths});
        for(const type of ['extensions','skills','prompts','themes'])for(const resource of paths[type]) {
            if(!resource.enabled||resource.path.startsWith('builtin:'))continue;
            fingerprint(resource.path,true);
            if(resource.metadata.packageRoot) {
                fingerprint(resource.metadata.packageRoot,true);
                // Installed dependencies are part of the executable package.
                const dependencies=join(resource.metadata.packageRoot,'node_modules');
                if(existsSync(dependencies))for(const name of readdirSync(dependencies).sort())fingerprint(join(dependencies,name),true);
            }
        }
    }
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
    return {node:process.version,resolved,resources:[...resources].sort(([a],[b])=>a.localeCompare(b))};
}

export function snapshotResources(sources,expected) {
    const records=[];
    for(const source of sources)for(const entry of source.entries) {
        if(entry.type!=='custom'||entry.customType!=='rollback-snapshot')continue;
        const data=entry.data;
        if(!data?.repoRoot||!/^refs\/pi\/rollback\//.test(data.ref??'')||!/^[a-f0-9]{40,64}$/.test(data.tree??''))throw Error('Unsupported rollback snapshot resource');
        const git=(...args)=>execFileSync('git',['-C',data.repoRoot,...args],{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
        const gitDirectory=realpathSync(git('rev-parse','--absolute-git-dir'));
        const retained=expected?.find(r=>r.gitDirectory===gitDirectory&&r.ref===data.ref&&r.tree===data.tree);
        const commit=retained?.commit??git('rev-parse','--verify',`${data.ref}^{commit}`);
        if(git('rev-parse','--verify',`${commit}^{tree}`)!==data.tree)throw Error(`Rollback snapshot tree changed: ${data.ref}`);
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
