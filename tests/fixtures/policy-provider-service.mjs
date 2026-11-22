import { servePolicyModel } from '../../../pi/packages/pi-integration/src/policy-model-service.ts';
await servePolicyModel({
 getModel(provider, id) {
  if (provider !== 'fake' || id !== 'small') throw Error('unexpected model');
  return { provider, id };
 },
 async complete(_model, context, options) {
  if (options.signal.aborted || options.maxRetries !== 0 || options.maxTokens !== 4) throw Error('invalid options');
  return {stopReason:'stop', content:[{type:'text',text:context.messages[0].content}]};
 },
});
