import { memo } from 'react'
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import {
  roleLabel,
  speculationKindLabel,
  speculationReason,
  statusLabel,
  type BranchState,
  type ChanceState,
} from '../fold'
import { ReasonIcon, StatusIcon } from '../icons'
import { settlementSummary } from '../treeLayout'
import { useAppStore } from '../store'

export interface BranchHeaderData extends Record<string, unknown> {
  branch: BranchState
  chance: ChanceState | null
  isAuthorityPath: boolean
  isLive: boolean
  callCount: number
}

export type BranchHeaderFlowNode = Node<BranchHeaderData, 'branchHeader'>

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function BranchNodeView({ data }: NodeProps<BranchHeaderFlowNode>) {
  const { branch, chance, isAuthorityPath, isLive } = data
  const role = roleLabel(branch.role)
  const reason = speculationKindLabel(branch.speculationKind)
  const settlement = settlementSummary(branch)
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
      aria-label={`分支 ${branch.branchId}，${role}，${statusLabel(branch.status)}，投机原因 ${reason}，${data.callCount} 次调用${
        settlement ? `，${settlement}` : ''
      }${chance?.toolName ? `，Chance 工具 ${chance.toolName}` : ''}`}
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
      <span
        className={`head-reason${
          branch.speculationKind === null ? ' head-reason-unreported' : ''
        }`}
        aria-hidden="true"
        title={`投机原因：${reason}`}
      >
        <ReasonIcon
          reason={speculationReason(branch.speculationKind)}
          size={16}
        />
      </span>
      {settlement ? (
        <span
          className={`head-settlement head-settlement-${
            branch.settlements.at(-1)?.hit ? 'hit' : 'miss'
          }`}
          aria-hidden="true"
          title={`结算：${settlement}`}
        >
          {branch.settlements.at(-1)?.hit ? '✓' : '✕'}
        </span>
      ) : null}
    </div>
  )
}

export const BranchNode = memo(BranchNodeView)
