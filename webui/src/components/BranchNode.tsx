import { memo, type CSSProperties } from 'react'
import { type Node, type NodeProps } from '@xyflow/react'
import type { BranchState, ChanceState } from '../fold'
import { FIXED_GEOMETRY, branchTone, type BranchGeometry, type CallGroup, type TreeTuning } from '../tree'
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
  const tone=branchTone(branch, isAuthorityPath, isCurrentAuthority)
  const open=(group?:CallGroup)=>openBranch(branch.branchId,group?{kind:group.kind,actionId:group.events[0]?.action_id??null,sequences:group.events.map(e=>e.sequence)}:undefined)
  const callTarget = (group: CallGroup) => ({
    role: 'button',
    tabIndex: 0,
    'aria-label': `${group.kind === 'model' ? '模型调用' : '工具调用'}：事件 ${group.firstSequence}`,
    'data-tree-call': group.id,
    onClick: (event: React.MouseEvent<SVGGElement>) => { event.stopPropagation(); open(group) },
    onKeyDown: (event: React.KeyboardEvent<SVGGElement>) => {
      if (event.key !== 'Enter' && event.key !== ' ') return
      event.preventDefault()
      event.stopPropagation()
      if (!event.repeat) open(group)
    },
  })
  const branchEntry = { onClick: () => open() }

  return <div className="tree-branch" data-branch-id={branch.branchId} data-tree-tone={tone}
    data-trunk-points={JSON.stringify(g.pathPoints)} data-trunk-widths={JSON.stringify(g.trunkWidths)}
    data-initial-angle={g.initialAngle*180/Math.PI}
    style={{width:g.width,height:g.height}}>
    <svg className="tree-svg" width={g.width} height={g.height} viewBox={`0 0 ${g.width} ${g.height}`}>
      <g transform={`translate(${g.offsetX} ${g.offsetY})`} className={`tree-ink tree-ink-${tone}`}>
        {g.connection ? <path className="tree-trunk tree-root-connection" d={g.connection.d}
          data-root-width={g.connection.rootWidth} {...branchEntry} /> : null}
        {g.connection ? null : g.calls.length
          ? <path className="tree-trunk" d={g.trunk.d} {...branchEntry}
              style={{ stroke: g.trunk.stroke ? 'currentColor' : 'none', strokeWidth: g.trunk.stroke }} />
          : <g transform={g.bud.transform}>
              <ellipse className="tree-empty-bud" cx={g.bud.ellipse.cx} cy={g.bud.ellipse.cy} rx={g.bud.ellipse.rx} ry={g.bud.ellipse.ry}
                {...branchEntry}>
                <title>空 Branch {branch.branchId}</title>
              </ellipse>
            </g>}
        {g.sections.map(section=>{
          const p=section
          const shape=p.shape!
          return <g key={section.id} className="tree-twig-group" data-anchor-x={shape.anchor.x} data-anchor-y={shape.anchor.y}
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
                  <path className="tree-leaf-visual" style={{strokeWidth:leaf.stroke,opacity:FIXED_GEOMETRY.leafOpacity}} data-length={leaf.size} data-width={leaf.width}
                    d={leaf.outline.d} />
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
      </g>
      <g transform={`translate(${g.offsetX} ${g.offsetY})`}>
        {childForks.map(f => {
          if (f.radius <= 0) return null
          const childId = f.candidateBranchIds.length === 1 ? f.candidateBranchIds[0] : undefined
          return <circle key={f.chance?.chanceId} className="tree-chance" data-parent-branch={branch.branchId}
            data-chance-id={f.chance?.chanceId} data-branch-id={childId} data-factual-y={f.y}
            cx={f.x} cy={f.y} r={f.radius}
            style={{ '--chance-radius': `${f.radius}px`, cursor: childId ? 'pointer' : 'default' } as CSSProperties}
            onClick={childId ? () => openBranch(childId) : undefined}>
            <title>Chance {f.chance?.chanceId}</title>
          </circle>
        })}
      </g>
    </svg>
    </div>
}
export const BranchNode=memo(BranchNodeView)
