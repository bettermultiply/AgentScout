import { memo } from 'react'
import type { Node, NodeProps } from '@xyflow/react'
import type { BranchState } from '../fold'
import { leafKindLabel, type LeafGroup } from '../treeLayout'
import { useAppStore } from '../store'

export interface LeafData extends Record<string, unknown> {
  branch: BranchState
  group: LeafGroup
  isAuthorityPath: boolean
}

export type LeafFlowNode = Node<LeafData, 'leaf'>

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function LeafNodeView({ data }: NodeProps<LeafFlowNode>) {
  const { branch, group } = data
  const open = () => useAppStore.getState().openBranch(branch.branchId)
  const count = group.events.length > 1 ? `，连续 ${group.events.length} 条` : ''

  return (
    <div
      className={[
        'tree-leaf',
        `leaf-${group.kind}`,
        `tone-${group.tone}`,
        isAbandoned(branch.status) ? 'leaf-abandoned' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      tabIndex={0}
      role="button"
      aria-label={`${leafKindLabel(group.kind)}，事件 ${group.firstSequence}${
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

export const LeafNode = memo(LeafNodeView)
