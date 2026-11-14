import { useState } from 'react'
import { DEFAULT_TUNING, TUNE_FIELDS, normalizeTuning, type TreeTuning } from '../tree'


/** Temporary drawing-parameter panel; not part of the product surface. */
export function TreeTuningPanel({ tuning, onChange, onClose }: { tuning: TreeTuning; onChange: (next: TreeTuning) => void; onClose: () => void }) {
  const [width, setWidth] = useState(360)

  const update = (key: keyof TreeTuning, value: number, scale = 1) =>
    onChange(normalizeTuning({ ...tuning, [key]: value * scale }))
  const groups = [...new Set(TUNE_FIELDS.map(field => field.group))]

  return (
    <div id="tree-tuning-panel" className="tree-tune-panel" style={{width, maxWidth:'calc(100% - 24px)'}} role="dialog" aria-label="临时调参">
      <div className="tree-tune-resize" role="separator" aria-label="调整调参面板宽度" aria-orientation="vertical" tabIndex={0}
        onKeyDown={e=>{if(e.key==='ArrowRight')setWidth(w=>Math.min(900,w+20));if(e.key==='ArrowLeft')setWidth(w=>Math.max(260,w-20))}}
        onPointerDown={e=>{e.currentTarget.setPointerCapture(e.pointerId)}}
        onPointerMove={e=>{if(e.currentTarget.hasPointerCapture(e.pointerId)){const left=e.currentTarget.parentElement!.getBoundingClientRect().left;setWidth(Math.max(260,Math.min(900,e.clientX-left)))}}}
      />
      <header>
        <strong>临时调参</strong>
        <div>
          <button type="button" onClick={() => onChange(DEFAULT_TUNING)}>重置</button>
          <button type="button" aria-label="收起调参面板" onClick={onClose}>收起</button>
        </div>
      </header>
      {groups.map(group => <fieldset key={group} className="tree-tune-group">
        <legend>{group}</legend>
        {group === '整体' ? <p className="tree-tune-note">节点局部原点仅作树枝、叶茎、叶片的共同坐标参考，不改变形态或世界位置。</p> : null}
        {TUNE_FIELDS.filter(field => field.group === group && (!field.visibleWhen || tuning[field.visibleWhen.key] === field.visibleWhen.value)).map((field) => field.mode ? (
          <label key={field.key} className="tree-tune-row">
            <span>{field.label}</span>
            <select aria-label={field.label} value={tuning[field.key]} onChange={event => update(field.key, Number(event.target.value))}>
              <option value={0}>固定值</option>
              <option value={1}>范围随机</option>
            </select>
          </label>
        ) : (
        <label key={field.key} className="tree-tune-row">
          <span>{field.label}</span>
          <input type="number" aria-label={field.label} min={field.min} max={field.max} step={field.step}
            key={`${field.key}-${tuning[field.key]}`} defaultValue={tuning[field.key]/(field.scale??1)}
            onBlur={e=>{
              if(e.target.value!=='')update(field.key,Number(e.target.value),field.scale)
              e.target.value=String(tuning[field.key]/(field.scale??1))
            }}
            onKeyDown={e=>{if(e.key==='Enter')e.currentTarget.blur()}} />
          <input
            type="range"
            aria-label={`${field.label}滑杆`}
            min={field.min}
            max={field.max}
            step={field.step}
            value={field.scale ? tuning[field.key] / field.scale : tuning[field.key]}
            onChange={(event) => update(field.key, Number(event.target.value), field.scale)}
          />
        </label>
        ))}
      </fieldset>)}
      <p className="tree-tune-note">参数自动保存在当前浏览器本地，刷新或再次打开时恢复。</p>
    </div>
  )
}
