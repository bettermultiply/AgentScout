import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { loadModelRuntime } from './model-runtime.ts';

const root = process.env.ASCOUT_PI_ROOT;
if (!root) throw Error('ASCOUT_PI_ROOT is required');
const args = process.argv.slice(2);
if (args.length && (args.length !== 2 || args[0] !== '--models' || !args[1])) {
    throw Error('usage: policy-provider.ts [--models <path>]');
}
const require = createRequire(resolve(root, 'package.json'));
const policyModelServicePath = resolve(root, 'packages/pi-integration/src/policy-model-service.ts');
let servePolicyModel;
try {
    ({ servePolicyModel } = require(policyModelServicePath));
} catch (error) {
    const reason = error instanceof Error ? `: ${error.message}` : '';
    throw Error(`Failed to load Pi policy model service at ${policyModelServicePath}${reason}`, {cause: error});
}
const { ModelRuntime } = require(resolve(root, 'packages/coding-agent/src/index.ts'));
try {
    const { runtime } = await loadModelRuntime(() => ModelRuntime.create(), args[1]);
    await servePolicyModel(runtime);
} catch {
    // Provider configuration and error messages may contain credentials.
    process.exitCode = 1;
}
