import { memo, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import {
  authorityStateLabel,
  branchTokenTotal,
  chanceStageLabel,
  latestModel,
  roleLabel,
  speculationKindLabel,
  statusLabel,
  type BranchState,
  type ChanceState,
} from '../fold'
import { formatTokens } from '../format'
import { ReasonIcon, StatusIcon } from '../icons'
import {
  HEAD_CENTER_Y,
  HEAD_SIZE,
  LEAD_REACH,
  NODE_HALF_WIDTH,
  NODE_WIDTH,
  TRUNK_WIDTH,
  callDetailLines,
  callKindLabel,
  settlementLines,
  settlementSummary,
  type BranchGeometry,
  type BranchSection,
  type CallGroup,
  type TwigLeafPoint,
} from '../treeLayout'
import { useAppStore } from '../store'

export interface BranchData extends Record<string, unknown> {
  branch: BranchState
  chance: ChanceState | null
  geometry: BranchGeometry
  isAuthorityPath: boolean
  isCurrentAuthority: boolean
  isLive: boolean
  /** Absolute y where the authority child forks; null when this is the tip. */
  authorityForkY: number | null
}

export type BranchFlowNode = Node<BranchData, 'branch'>

type StrokeTone = 'authority' | 'candidate' | 'abandoned'

interface HoverState {
  group: CallGroup | null
  x: number
  y: number
  below: boolean
}

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function strokeTone(authority: boolean, abandoned: boolean): StrokeTone {
  if (abandoned) return 'abandoned'
  return authority ? 'authority' : 'candidate'
}

function chanceLines(chance: ChanceState | null): string[] {
  if (!chance) return []
  return [
    `Chance 工具：${chance.toolName || '未上报'}`,
    `Chance 阶段：${chanceStageLabel(chance.stage)}`,
    `权威状态：${authorityStateLabel(chance.authorityState)}`,
    `父 Chance：${chance.parentChanceId ?? '未上报'}`,
  ]
}

function HoverCard({
  branch,
  chance,
  group,
  x,
  y,
  below,
  onEnter,
  onLeave,
}: {
  branch: BranchState
  chance: ChanceState | null
  group: CallGroup | null
  x: number
  y: number
  below: boolean
  onEnter: () => void
  onLeave: () => void
}) {
  const tokens = branchTokenTotal(branch)
  const model = latestModel(branch)
  const calls = group ? callDetailLines(group) : []
  const settlements = settlementLines(branch)

  return (
    <div
      className={`node-tooltip node-tooltip-float${below ? ' tooltip-below' : ''}`}
      style={{ left: x, top: y }}
      role="tooltip"
      onMouseEnter={onEnter}
      onMouseLeave={onLeave}
    >
      <span className="node-tooltip-id">{branch.branchId}</span>
      <span>
        {roleLabel(branch.role)} · {statusLabel(branch.status)}
      </span>
      {group ? (
        <>
          <span className="node-tooltip-section">
            {callKindLabel(group.kind)}
            {group.events.length > 1 ? ` · 连续 ${group.events.length} 条` : ''}
          </span>
          <span className="node-tooltip-id">
            事件 #{group.firstSequence}
            {group.lastSequence !== group.firstSequence
              ? `–#${group.lastSequence}`
              : ''}
          </span>
          <div className="node-tooltip-calls">
            {calls.map((call) => (
              <span className="node-tooltip-call" key={call.sequence}>
                #{call.sequence} {call.text}
              </span>
            ))}
          </div>
          <span className="node-tooltip-hint">点击定位并高亮详情中的调用</span>
        </>
      ) : (
        <>
          <span>模型：{model ?? '未上报'}</span>
          <span>
            token：{tokens.known ? formatTokens(tokens.value) : '未上报'}
            {tokens.known && !tokens.complete ? '（部分）' : ''}
          </span>
          <span>投机原因：{speculationKindLabel(branch.speculationKind)}</span>
          {branch.chanceId ? (
            <>
              <span className="node-tooltip-id">Chance：{branch.chanceId}</span>
              {chance ? (
                chanceLines(chance).map((line) => (
                  <span key={line}>{line}</span>
                ))
              ) : (
                <span>Chance 详情：未上报</span>
              )}
            </>
          ) : (
            <span>Chance：无（初始权威）</span>
          )}
          {settlementSummary(branch) ? (
            <span className="node-tooltip-section">
              {settlements.map((line) => (
                <span className="node-tooltip-call" key={line}>
                  {line}
                </span>
              ))}
            </span>
          ) : null}
          <span className="node-tooltip-section">
            {branch.modelUsages.length + branch.tools.length} 次调用 ·{' '}
            {branch.events.length} 个事件
          </span>
          <span className="node-tooltip-hint">点击进入分支详情顶部</span>
        </>
      )}
    </div>
  )
}

function LeafButton({
  branchId,
  group,
  point,
  side,
  tone,
  abandoned,
  onHover,
  onLeave,
}: {
  branchId: string
  group: CallGroup
  point: TwigLeafPoint
  side: -1 | 1
  tone: StrokeTone
  abandoned: boolean
  onHover: (event: React.MouseEvent<Element>, group: CallGroup) => void
  onLeave: () => void
}) {
  const open = () =>
    useAppStore.getState().openBranch(branchId, {
      kind: 'tool',
      actionId: group.events[0]?.action_id ?? null,
      sequences: group.events.map((event) => event.sequence),
    })

  return (
    <button
      type="button"
      className={[
        'tree-leaf',
        `tone-${group.tone}`,
        `leaf-${tone}`,
        side > 0 ? 'leaf-side-right' : 'leaf-side-left',
        abandoned ? 'leaf-abandoned' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      style={{ left: point.x, top: point.y }}
      aria-label={`工具调用：事件 ${group.firstSequence}${
        group.lastSequence !== group.firstSequence
          ? ` 到 ${group.lastSequence}`
          : ''
      }`}
      onMouseEnter={(event) => onHover(event, group)}
      onMouseLeave={onLeave}
      onClick={(event) => {
        event.stopPropagation()
        open()
      }}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          open()
        }
      }}
    >
      <span
        className="leaf-shape"
        style={{ ['--leaf-angle' as string]: `${point.angle}deg` }}
        aria-hidden="true"
      />
      {group.events.length > 1 ? (
        <span className="leaf-count" aria-hidden="true">
          ×{group.events.length}
        </span>
      ) : null}
    </button>
  )
}

function twigLabel(group: CallGroup): string {
  const models = [
    ...new Set(
      group.events.map((event) =>
        event.event === 'model_usage' ? event.model : '',
      ),
    ),
  ].join('、')
  return `模型调用：${models || '未上报'}，事件 ${group.firstSequence}${
    group.lastSequence !== group.firstSequence
      ? ` 到 ${group.lastSequence}`
      : ''
  }`
}

function BranchNodeView({ data }: NodeProps<BranchFlowNode>) {
  const {
    branch,
    chance,
    geometry,
    isAuthorityPath,
    isCurrentAuthority,
    isLive,
    authorityForkY,
  } = data
  const abandoned = isAbandoned(branch.status)
  const openBranch = useAppStore((store) => store.openBranch)
  const [hover, setHover] = useState<HoverState | null>(null)
  const closeTimer = useRef<number | null>(null)

  const cancelClose = () => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current)
      closeTimer.current = null
    }
  }

  const scheduleClose = () => {
    cancelClose()
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null
      setHover(null)
    }, 160)
  }

  useEffect(() => cancelClose, [])

  useEffect(() => {
    if (!hover) return
    const onScroll = () => setHover(null)
    window.addEventListener('scroll', onScroll, true)
    return () => window.removeEventListener('scroll', onScroll, true)
  }, [hover])

  const show = (event: React.MouseEvent<Element>, group: CallGroup | null) => {
    cancelClose()
    const canvas = event.currentTarget.closest('.tree-canvas')
    const rect = canvas?.getBoundingClientRect()
    const left = rect?.left ?? 0
    const right = rect?.right ?? window.innerWidth
    const top = rect?.top ?? 0
    setHover({
      group,
      x: Math.min(Math.max(event.clientX, left + 170), right - 170),
      y: event.clientY,
      below: event.clientY - top < 170,
    })
  }

  const localFork =
    authorityForkY === null ? null : authorityForkY - geometry.y
  const trunkTop = HEAD_CENTER_Y
  const trunkBottom = geometry.height - 2

  const authorityAt = (localY: number): boolean => {
    if (!isAuthorityPath) return false
    if (isCurrentAuthority || localFork === null) return true
    return localY <= localFork
  }

  const authorityThrough = (section: BranchSection): boolean =>
    authorityAt(section.y)

  const segments: Array<{ from: number; to: number; tone: StrokeTone }> = []
  if (abandoned) {
    segments.push({ from: trunkTop, to: trunkBottom, tone: 'abandoned' })
  } else if (!isAuthorityPath) {
    segments.push({ from: trunkTop, to: trunkBottom, tone: 'candidate' })
  } else if (isCurrentAuthority || localFork === null) {
    segments.push({ from: trunkTop, to: trunkBottom, tone: 'authority' })
  } else {
    const split = Math.min(Math.max(localFork, trunkTop), trunkBottom)
    if (split > trunkTop) {
      segments.push({ from: trunkTop, to: split, tone: 'authority' })
    }
    if (trunkBottom > split) {
      segments.push({ from: split, to: trunkBottom, tone: 'candidate' })
    }
  }

  const openHead = () => openBranch(branch.branchId)
  const settlement = settlementSummary(branch)
  const reason = speculationKindLabel(branch.speculationKind)

  return (
    <div
      className="tree-branch"
      style={{ width: NODE_WIDTH, height: geometry.height }}
    >
      <svg
        className="tree-svg"
        width={NODE_WIDTH}
        height={geometry.height}
        viewBox={`0 0 ${NODE_WIDTH} ${geometry.height}`}
        focusable="false"
      >
        {segments.map((segment) => (
          <g key={`${segment.from}-${segment.to}`}>
            <line
              className={`tree-trunk tree-trunk-${segment.tone}${
                isLive ? ' tree-trunk-live' : ''
              }`}
              x1={NODE_HALF_WIDTH}
              y1={segment.from}
              x2={NODE_HALF_WIDTH}
              y2={segment.to}
            />
            {segment.tone === 'candidate' ? (
              <line
                className="tree-trunk-core"
                x1={NODE_HALF_WIDTH}
                y1={segment.from}
                x2={NODE_HALF_WIDTH}
                y2={segment.to}
              />
            ) : null}
          </g>
        ))}

        {geometry.sections.map((section) => {
          const tone = strokeTone(authorityThrough(section), abandoned)
          if (!section.twig) {
            const leaf = section.leaves[0]
            if (!leaf) return null
            const x0 = NODE_HALF_WIDTH + section.side * (TRUNK_WIDTH / 2)
            const x1 =
              NODE_HALF_WIDTH +
              section.side * LEAD_REACH -
              section.side * 5
            return (
              <g key={section.id}>
                <path
                  className={`tree-stem tree-stem-${tone}`}
                  d={`M ${x0} ${section.y} Q ${
                    NODE_HALF_WIDTH + section.side * (LEAD_REACH * 0.55)
                  } ${section.y - 2} ${x1} ${section.y}`}
                />
              </g>
            )
          }

          const shape = section.shape
          if (!shape) return null
          return (
            <g
              key={section.id}
              className={[
                'tree-twig-group',
                `tree-twig-${tone}`,
                abandoned ? 'twig-abandoned' : '',
              ]
                .filter(Boolean)
                .join(' ')}
              tabIndex={0}
              role="button"
              aria-label={twigLabel(section.twig)}
              onMouseEnter={(event) => show(event, section.twig)}
              onMouseLeave={scheduleClose}
              onClick={(event) => {
                event.stopPropagation()
                if (!section.twig) return
                openBranch(branch.branchId, {
                  kind: 'model',
                  actionId: section.twig.events[0]?.action_id ?? null,
                  sequences: section.twig.events.map((event) => event.sequence),
                })
              }}
              onKeyDown={(event) => {
                if (
                  (event.key === 'Enter' || event.key === ' ') &&
                  section.twig
                ) {
                  event.preventDefault()
                  openBranch(branch.branchId, {
                    kind: 'model',
                    actionId: section.twig.events[0]?.action_id ?? null,
                    sequences: section.twig.events.map(
                      (event) => event.sequence,
                    ),
                  })
                }
              }}
            >
              <path className="tree-twig-visual" d={shape.d} />
              <path className="tree-twig-hit" d={shape.d} />
              <circle
                className="tree-twig-tip"
                cx={shape.tip.x}
                cy={shape.tip.y}
                r={2.7}
              />
            </g>
          )
        })}
      </svg>

      {geometry.sections.map((section) => {
        if (section.twig || !section.leaves[0]) return null
        const tone = strokeTone(authorityThrough(section), abandoned)
        return (
          <LeafButton
            key={section.id}
            branchId={branch.branchId}
            group={section.leaves[0]}
            point={{
              x: NODE_HALF_WIDTH + section.side * LEAD_REACH,
              y: section.y,
              angle: section.side > 0 ? -18 : 18,
            }}
            side={section.side}
            tone={tone}
            abandoned={abandoned}
            onHover={(event, group) => show(event, group)}
            onLeave={scheduleClose}
          />
        )
      })}

      {geometry.sections.map((section) => {
        if (!section.twig || !section.shape) return null
        return (
          <span key={`twig:${section.id}`} className="tree-twig-parts">
            {section.leaves.map((leaf, index) => {
              const point = section.shape?.leaves[index]
              if (!point) return null
              const tone = strokeTone(authorityAt(point.y), abandoned)
              return (
                <LeafButton
                  key={leaf.id}
                  branchId={branch.branchId}
                  group={leaf}
                  point={point}
                  side={section.side}
                  tone={tone}
                  abandoned={abandoned}
                  onHover={(event, group) => show(event, group)}
                  onLeave={scheduleClose}
                />
              )
            })}
            {section.twig.events.length > 1 ? (
              <span
                className="twig-count"
                style={{
                  left: section.shape.tip.x,
                  top: section.shape.tip.y - 9,
                }}
                aria-hidden="true"
              >
                ×{section.twig.events.length}
              </span>
            ) : null}
          </span>
        )
      })}

      <button
        type="button"
        className={[
          'branch-head',
          isAuthorityPath ? 'head-authority' : 'head-candidate',
          abandoned ? 'head-abandoned' : '',
          isLive ? 'head-live' : '',
        ]
          .filter(Boolean)
          .join(' ')}
        style={{
          left: NODE_HALF_WIDTH - HEAD_SIZE / 2,
          top: 0,
          width: HEAD_SIZE,
          height: HEAD_SIZE,
        }}
        aria-label={`分支 ${branch.branchId}，${roleLabel(branch.role)}，${statusLabel(
          branch.status,
        )}，投机原因 ${reason}，${
          branch.modelUsages.length + branch.tools.length
        } 次调用${settlement ? `，${settlement}` : ''}${
          chance?.toolName ? `，Chance 工具 ${chance.toolName}` : ''
        }`}
        onMouseEnter={(event) => show(event, null)}
        onMouseLeave={scheduleClose}
        onClick={(event) => {
          event.stopPropagation()
          openHead()
        }}
      >
        <StatusIcon status={branch.status} size={8} />
        <span
          className={`head-reason${
            branch.speculationKind === null ? ' head-reason-unreported' : ''
          }`}
          aria-hidden="true"
        >
          <ReasonIcon
            reason={
              branch.speculationKind === 'tool_execution'
                ? 'tool'
                : branch.speculationKind === 'model_inference'
                  ? 'model'
                  : 'other'
            }
            size={6}
          />
        </span>
        {settlement ? (
          <span
            className={`head-settlement head-settlement-${
              branch.settlements.at(-1)?.hit ? 'hit' : 'miss'
            }`}
            aria-hidden="true"
          >
            {branch.settlements.at(-1)?.hit ? '✓' : '✕'}
          </span>
        ) : null}
      </button>

      <Handle
        type="target"
        position={Position.Top}
        id="in"
        className="tree-handle"
        style={{
          left: NODE_HALF_WIDTH,
          top: 0,
          transform: 'translate(-50%, -50%)',
        }}
      />
      {[...geometry.forks].map(([childId, forkY]) => (
        <Handle
          key={childId}
          type="source"
          position={Position.Bottom}
          id={`fork:${childId}`}
          className="tree-handle"
          style={{
            left: NODE_HALF_WIDTH,
            top: forkY - geometry.y,
            transform: 'translate(-50%, -50%)',
          }}
        />
      ))}

      {hover
        ? createPortal(
            <HoverCard
              branch={branch}
              chance={chance}
              group={hover.group}
              x={hover.x}
              y={hover.y}
              below={hover.below}
              onEnter={cancelClose}
              onLeave={scheduleClose}
            />,
            document.body,
          )
        : null}
    </div>
  )
}

export const BranchNode = memo(BranchNodeView)
