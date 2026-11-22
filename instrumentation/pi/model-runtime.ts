import { readFileSync } from 'node:fs';

export async function loadModelRuntime(create: () => Promise<any>, modelsPath?: string) {
    const runtime = await create();
    const additionalModels = modelsPath ? JSON.parse(readFileSync(modelsPath, 'utf8')) : undefined;
    for (const [id, config] of Object.entries(additionalModels?.providers ?? {})) {
        if (runtime.getProvider(id)) throw Error(`Additional provider already exists: ${id}`);
        runtime.registerProvider(id, config);
    }
    return { runtime, additionalModels };
}
