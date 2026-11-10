import { memo } from 'react'
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import {
  roleLabel,
  speculationKindLabel,
  speculationReason,
  statusLabel,
  type BranchState,
} from '../fold'
import { ReasonIcon, StatusIcon } from '../icons'
import { useAppStore } from '../store'
import type { BranchObservationEvent } from '../types'

export interface BranchNodeData extends Record<string, unknown> {
  branch: BranchState
  event: BranchObservationEvent
  isAuthorityPath: boolean
  isLatest: boolean
}

export type BranchFlowNode = Node<BranchNodeData, 'branchEvent'>

function isLive(status: BranchState['status']): boolean {
  return (
    status === 'running' ||
    status === 'awaiting_validation' ||
    status === 'promoting'
  )
}

function BranchNodeView({ data }: NodeProps<BranchFlowNode>) {
  const { branch, isAuthorityPath, isLatest } = data
  const role = roleLabel(branch.role)
  const reason = speculationKindLabel(branch.speculationKind)
  const open = () => useAppStore.getState().openBranch(branch.branchId)

  return (
    <div
      className={[
        'tree-node',
        isAuthorityPath ? 'node-authority' : 'node-candidate',
        `status-${branch.status}`,
        branch.status === 'discarded' || branch.status === 'cleaned'
          ? 'node-abandoned'
          : '',
        isLatest && isLive(branch.status) ? 'node-live' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      tabIndex={0}
      role="button"
      aria-label={`分支 ${branch.branchId}，${role}，${statusLabel(branch.status)}，投机原因 ${reason}`}
      onKeyDown={(keyboard) => {
        if (keyboard.key === 'Enter' || keyboard.key === ' ') {
          keyboard.preventDefault()
          open()
        }
      }}
    >
      <Handle type="target" position={Position.Top} className="tree-handle" />
      <span className="node-status" aria-hidden="true">
        <StatusIcon status={branch.status} size={16} />
      </span>
      <span className="node-reason" aria-hidden="true">
        <ReasonIcon reason={speculationReason(branch.speculationKind)} size={11} />
      </span>
      <Handle type="source" position={Position.Bottom} className="tree-handle" />
    </div>
  )
}

export const BranchNode = memo(BranchNodeView)
