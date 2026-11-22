// @ts-nocheck
import {readFileSync, openSync, closeSync, writeFileSync, fsyncSync, renameSync, unlinkSync} from 'node:fs';
import {dirname} from 'node:path';
import {randomUUID, createHash} from 'node:crypto';

export const checksum = value => createHash('sha256').update(value).digest('hex');

export function save(file, value) {
    const bytes=JSON.stringify(value),temporary=`${file}.${randomUUID()}.tmp`;
    let descriptor;
    try {
        descriptor=openSync(temporary,'wx',0o600);
        writeFileSync(descriptor,bytes);fsyncSync(descriptor);closeSync(descriptor);descriptor=undefined;
        if(readFileSync(temporary,'utf8')!==bytes)throw Error('Recovery write verification failed');
        renameSync(temporary,file);
        const directory=openSync(dirname(file),'r');try{fsyncSync(directory)}finally{closeSync(directory)}
    } catch(error) {
        if(descriptor!==undefined)closeSync(descriptor);
        try{unlinkSync(temporary)}catch{}
        throw error;
    }
}

export function load(file) {
    const value=JSON.parse(readFileSync(file,'utf8'));
    if(value.version!==1)throw Error('Unsupported recovery version');
    return value;
}
