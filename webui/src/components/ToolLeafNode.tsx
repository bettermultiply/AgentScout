import { memo } from 'react'
import type { Node, NodeProps } from '@xyflow/react'
import type { BranchState } from '../fold'
import { toolName, type CallGroup, type CallSide } from '../treeLayout'
import { useAppStore } from '../store'

export interface ToolLeafData extends Record<string, unknown> {
  branch: BranchState
  group: CallGroup
  isAuthorityPath: boolean
  side: CallSide
}

export type ToolLeafFlowNode = Node<ToolLeafData, 'toolLeaf'>

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function ToolLeafView({ data }: NodeProps<ToolLeafFlowNode>) {
  const { branch, group, side } = data
  const open = () =>
    useAppStore.getState().openBranch(branch.branchId, {
      kind: 'tool',
      actionId: group.events[0]?.action_id ?? null,
      sequences: group.events.map((event) => event.sequence),
    })
  const count = group.events.length > 1 ? `，连续 ${group.events.length} 条` : ''
  const names = [...new Set(
    group.events.map((event) => (event.event === 'tool_finished' ? toolName(event) : '')),
  )].join('、')

  return (
    <div
      className={[
        'tree-leaf',
        'leaf-tool',
        `tone-${group.tone}`,
        side === 0 ? 'leaf-on-trunk' : side > 0 ? 'leaf-side-right' : 'leaf-side-left',
        isAbandoned(branch.status) ? 'leaf-abandoned' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      tabIndex={0}
      role="button"
      aria-label={`工具调用：${names}，事件 ${group.firstSequence}${
        group.lastSequence !== group.firstSequence
          ? ` 到 ${group.lastSequence}`
          : ''
      }${count}`}
      onKeyDown={(keyboard) => {
        if (keyboard.key === 'Enter' || keyboard.key === ' ') {
          keyboard.preventDefault()
          open()
        }
      }}
    >
      <span className="leaf-shape" aria-hidden="true" />
      {group.events.length > 1 ? (
        <span className="leaf-count" aria-hidden="true">
          ×{group.events.length}
        </span>
      ) : null}
    </div>
  )
}

export const ToolLeafNode = memo(ToolLeafView)
