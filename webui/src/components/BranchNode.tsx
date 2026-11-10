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

export interface BranchHeaderData extends Record<string, unknown> {
  branch: BranchState
  isAuthorityPath: boolean
  isLive: boolean
  leafCount: number
}

export type BranchHeaderFlowNode = Node<BranchHeaderData, 'branchHeader'>

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function BranchNodeView({ data }: NodeProps<BranchHeaderFlowNode>) {
  const { branch, isAuthorityPath, isLive } = data
  const role = roleLabel(branch.role)
  const reason = speculationKindLabel(branch.speculationKind)
  const open = () => useAppStore.getState().openBranch(branch.branchId)

  return (
    <div
      className={[
        'branch-head',
        isAuthorityPath ? 'head-authority' : 'head-candidate',
        isAbandoned(branch.status) ? 'head-abandoned' : '',
        isLive ? 'head-live' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      tabIndex={0}
      role="button"
      aria-label={`分支 ${branch.branchId}，${role}，${statusLabel(branch.status)}，投机原因 ${reason}，${data.leafCount} 个叶子`}
      onKeyDown={(keyboard) => {
        if (keyboard.key === 'Enter' || keyboard.key === ' ') {
          keyboard.preventDefault()
          open()
        }
      }}
    >
      <Handle
        type="target"
        position={Position.Top}
        id="header-in"
        className="tree-handle"
      />
      <span className="head-status" aria-hidden="true">
        <StatusIcon status={branch.status} size={24} />
      </span>
      <span className="head-reason" aria-hidden="true">
        <ReasonIcon reason={speculationReason(branch.speculationKind)} size={18} />
      </span>
    </div>
  )
}

export const BranchNode = memo(BranchNodeView)
