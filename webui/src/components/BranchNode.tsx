import { memo, useState } from 'react'
import { createPortal } from 'react-dom'
import { type Node, type NodeProps } from '@xyflow/react'
import { type BranchState, type ChanceState, statusLabel } from '../fold'
import {
  HEAD_CENTER_Y,
  NODE_HALF_WIDTH,
  FIXED_GEOMETRY,
  branchTone,
  callDetailLines,
  type BranchGeometry,
  type CallGroup,
  type TreeTuning,
} from '../tree'
import { useAppStore } from '../store'

export interface BranchData extends Record<string, unknown> {
  branch: BranchState
  geometry: BranchGeometry
  tuning: TreeTuning
  isAuthorityPath: boolean
  isCurrentAuthority: boolean
  childForks: Array<{
    candidateBranchIds: string[]
    chance: ChanceState | null
    x: number
    y: number
    radius: number
  }>
}
export type BranchFlowNode = Node<BranchData, 'branch'>

function BranchNodeView({ data }: NodeProps<BranchFlowNode>) {
  const { branch, geometry:g, isAuthorityPath, isCurrentAuthority, childForks } = data
  const openBranch = useAppStore(store=>store.openBranch)
  const [hover,setHover] = useState<{x:number;y:number;group:CallGroup|null;chance:ChanceState|null}|null>(null)
  // Hover identity survives fresh observation payloads; its facts must not.
  const hoverGroup = hover?.group ? g.calls.find(call => call.id === hover.group?.id) ?? null : null
  const hoverFork = hover?.chance ? childForks.find(f => f.chance?.chanceId === hover.chance?.chanceId) : undefined
  const hoverChance = hoverFork?.chance
  // All ink is in one tree coordinate system; zoom scales dots and leaves too.
  const tone=branchTone(branch, isAuthorityPath, isCurrentAuthority)
  const aborted=tone==='aborted'
  const root=!branch.parentBranchId
  const show=(event:React.MouseEvent<Element>,group:CallGroup|null,chance:ChanceState|null=null)=>setHover({x:event.clientX,y:event.clientY,group,chance})
  const open=(group?:CallGroup)=>openBranch(branch.branchId,group?{kind:group.kind,actionId:group.events[0]?.action_id??null,sequences:group.events.map(e=>e.sequence)}:undefined)
  const callTarget = (group: CallGroup) => ({
    role: 'button',
    tabIndex: 0,
    'aria-label': `${group.kind === 'model' ? '模型调用' : '工具调用'}：事件 ${group.firstSequence}`,
    onMouseEnter: (event: React.MouseEvent<SVGGElement>) => show(event, group),
    onMouseLeave: () => setHover(null),
    onClick: (event: React.MouseEvent<SVGGElement>) => { event.stopPropagation(); open(group) },
    onKeyDown: (event: React.KeyboardEvent<SVGGElement>) => {
      if (event.key !== 'Enter' && event.key !== ' ') return
      event.preventDefault()
      event.stopPropagation()
      if (!event.repeat) open(group)
    },
  })
  const branchEntry = root ? {} : {
    onMouseEnter: (e: React.MouseEvent<SVGElement>) => show(e, null),
    onMouseLeave: () => setHover(null),
    onClick: () => open(),
  }

  return <div className="tree-branch" data-branch-id={branch.branchId} data-tree-tone={tone}
    data-trunk-points={JSON.stringify(g.pathPoints)} data-trunk-widths={JSON.stringify(g.trunkWidths)}
    data-initial-angle={g.initialAngle*180/Math.PI}
    style={{width:g.width,height:g.height}}>
    <svg className="tree-svg" width={g.width} height={g.height} viewBox={`0 0 ${g.width} ${g.height}`}>
      <g transform={`translate(${g.offsetX} ${g.offsetY})`} className={`tree-ink tree-ink-${tone}`}>
        {g.connection ? <path className="tree-trunk tree-root-connection" d={g.connection.d}
          data-root-width={g.connection.rootWidth}
          style={{stroke:'none',pointerEvents:'all',cursor:'pointer'}} onMouseEnter={e=>show(e,null)} onMouseLeave={()=>setHover(null)} onClick={()=>open()} /> : null}
        {g.connection ? null : g.calls.length
          ? <path className="tree-trunk" d={g.trunk.d} {...branchEntry}
              style={{ stroke: g.trunk.stroke ? 'currentColor' : 'none', strokeWidth: g.trunk.stroke,
                pointerEvents: root ? undefined : 'visiblePainted', cursor: root ? undefined : 'pointer' }} />
          : <g transform={g.bud.transform}>
              <ellipse className="tree-empty-bud" cx={g.bud.ellipse.cx} cy={g.bud.ellipse.cy} rx={g.bud.ellipse.rx} ry={g.bud.ellipse.ry}
                {...branchEntry} style={root ? undefined : { pointerEvents: 'visiblePainted', cursor: 'pointer' }}>
                <title>空 Branch {branch.branchId}</title>
              </ellipse>
            </g>}
        {g.sections.map(section=>{
          const p=section
          const shape=p.shape!
          // The stem belongs to the Branch and only the Branch. There is no
          // second "this call is below the authority fork" colour: that made
          // pink stems grow out of a white trunk with no pink Branch to explain
          // them.
          const sectionTone: typeof tone = aborted ? 'aborted' : tone
          return <g key={section.id} className={`tree-twig-group tree-ink-${sectionTone}`} data-anchor-x={shape.anchor.x} data-anchor-y={shape.anchor.y}
            data-center-x={p.x} data-center-y={p.y} data-half-width={p.width/2}>
            <g className="tree-twig-target" {...callTarget(section.twig)}>
              <path className="tree-twig-visual" data-length={shape.length} d={shape.outline!.d} />
              <circle className="tree-twig-visual" cx={shape.cap!.cx} cy={shape.cap!.cy} r={shape.cap!.r} />
              {section.leaves.length === 0 && <g transform={shape.transform}>
                <circle className="tree-model-tip" cx={shape.tip.x} cy={shape.tip.y} r={shape.tipRadius} />
              </g>}
            </g>
            {shape.leaves.length > 0 && <g transform={shape.transform}>
              {shape.leaves.map(leaf => {
                const tool = leaf.tool
                return <g key={tool.id} className={`tree-leaf-group tone-${tool.tone}`} transform={leaf.transform} {...callTarget(tool)}>
                  <path className="tree-leaf-petiole" style={{strokeWidth:leaf.petiole.stroke}} d={leaf.petiole.d} />
                  <ellipse className="tree-leaf-visual" style={{strokeWidth:leaf.stroke,opacity:FIXED_GEOMETRY.leafOpacity}} data-length={leaf.size} data-width={leaf.width}
                    cx={leaf.ellipse.cx} cy={leaf.ellipse.cy} rx={leaf.ellipse.rx} ry={leaf.ellipse.ry} />
                  <path className="tree-leaf-vein" style={{strokeWidth:leaf.vein.stroke}} d={leaf.vein.d} />
                </g>
              })}
            </g>}
          </g>
        })}
        {g.validationEnd ? <line className="tree-validation-end" x1={g.validationEnd.x1} y1={g.validationEnd.y1} x2={g.validationEnd.x2} y2={g.validationEnd.y2}
          stroke="#c48116" strokeWidth={2} style={{pointerEvents:'stroke'}}>
          <title>父枝止于此；Proposal 验证完成于 {new Date(g.validationEnd.validatedAtMs).toLocaleString()}</title>
        </line> : null}
        {root?<path className="tree-head-hit" d={`M ${NODE_HALF_WIDTH} ${HEAD_CENTER_Y} v ${12}`} strokeWidth={g.trunkWidths[0]} onMouseEnter={e=>show(e,null)} onClick={()=>open()} />:null}
      </g>
      {/* A Chance is an observed opportunity, not ink of the Branch it sits on,
          so it keeps its own colour and opacity even on an aborted trunk. */}
      <g transform={`translate(${g.offsetX} ${g.offsetY})`}>
        {childForks.map(f => {
          const childId = f.candidateBranchIds.length === 1 ? f.candidateBranchIds[0] : undefined
          return <circle key={f.chance?.chanceId} className="tree-chance" data-parent-branch={branch.branchId}
            data-chance-id={f.chance?.chanceId} data-branch-id={childId} data-factual-y={f.y}
            cx={f.x} cy={f.y} r={f.radius} style={{ cursor: childId ? 'pointer' : 'default' }}
            onMouseEnter={e => show(e, null, f.chance)} onMouseLeave={() => setHover(null)}
            onClick={childId ? () => openBranch(childId) : undefined}>
            <title>Chance {f.chance?.chanceId}</title>
          </circle>
        })}
      </g>
    </svg>
    {hover ? createPortal(
      <div className="node-tooltip node-tooltip-float tooltip-below" style={{ left: Math.min(hover.x, window.innerWidth - 350), top: hover.y + 10 }} role="tooltip">
        <span>{branch.branchId} · {statusLabel(branch.status)}</span>
        {hoverChance ? <span>Chance {hoverChance.chanceId} · {hoverChance.toolName ?? '未上报'}</span> : null}
        {hoverFork && hoverFork.candidateBranchIds.length > 1 ? <>
          <span>候选 Branch（点击枝或叶茎查看）</span>
          {hoverFork.candidateBranchIds.map(id => <span key={id}>{id}</span>)}
        </> : null}
        {hoverGroup ? callDetailLines(hoverGroup).map(c => <span key={c.sequence}>#{c.sequence} {c.text}</span>)
          : <span>{branch.modelUsages.length + branch.tools.length} 次调用</span>}
      </div>, document.body) : null}
    </div>
}
export const BranchNode=memo(BranchNodeView)
