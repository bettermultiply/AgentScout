import { useState } from 'react'
import { sendRequest } from '../api'
import type { UiPromptEvent } from '../types'

export function AgentPrompt({ prompt, connected }: { prompt: UiPromptEvent; connected: boolean }) {
  const [text, setText] = useState(prompt.prefill ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  async function answer(value: unknown, cancelled = false) {
    setBusy(true)
    setError('')
    try { await sendRequest({ action: 'ui_prompt_answer', prompt_id: prompt.prompt_id, answer: value, cancelled }) }
    catch (failure) { setError(String(failure)); setBusy(false) }
  }
  return <section className="agent-prompt" aria-label="Agent 提示">
    <strong>{prompt.title}</strong>
    {prompt.message_text !== null && <div className="message-text">{prompt.message_text}</div>}
    <fieldset disabled={busy || !connected}>
      {prompt.kind === 'confirm' && <><button onClick={() => void answer(true)}>是</button><button onClick={() => void answer(false)}>否</button></>}
      {prompt.kind === 'select' && prompt.options.map((option, index) => <button key={index} onClick={() => void answer(option)}>{option}</button>)}
      {(prompt.kind === 'input' || prompt.kind === 'editor') && <>
        {prompt.kind === 'editor'
          ? <textarea aria-label={prompt.title} value={text} onChange={event => setText(event.target.value)} />
          : <input aria-label={prompt.title} placeholder={prompt.placeholder ?? ''} value={text} onChange={event => setText(event.target.value)} />}
        <button onClick={() => void answer(text)}>提交</button>
      </>}
      <button onClick={() => void answer(null, true)}>取消</button>
    </fieldset>
    {error && <div role="alert">{error}</div>}
  </section>
}
