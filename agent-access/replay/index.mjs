/** Read-only native recording, consumed only at the model boundary. */
import { loadRecording } from './recording.mjs';
import { requestKey } from './normalize.mjs';

export function createReplay(run) {
  const {recording,inputs}=loadRecording(run);
  if(!inputs.length)throw Error('No consumed native user messages found in events.jsonl');
  const used=new Set();let failure=null;
  return {
    inputs,
    status() {return {total:recording.length,consumed:used.size,remaining:recording.length-used.size,failure};},
    next(messages) {
      const key=requestKey(messages);
      const index=recording.findIndex((r,i)=>r.key===key&&!used.has(i));
      if(index<0) {
        failure=`Missing or exhausted replay key ${key}; ${recording.length-used.size} recorded replies remain`;
        throw Error(failure);
      }
      used.add(index);return structuredClone(recording[index].reply);
    },
  };
}
