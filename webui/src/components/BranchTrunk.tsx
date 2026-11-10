import { memo } from 'react'
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react'
import type { BranchState } from '../fold'

export interface TrunkFork {
  childId: string
  /** Offset from the trunk top, in flow coordinates. */
  offset: number
}

export interface BranchTrunkData extends Record<string, unknown> {
  branch: BranchState
  isAuthorityPath: boolean
  isLive: boolean
  height: number
  forks: TrunkFork[]
}

export type BranchTrunkFlowNode = Node<BranchTrunkData, 'branchTrunk'>

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function BranchTrunkView({ data }: NodeProps<BranchTrunkFlowNode>) {
  const { branch, isAuthorityPath, isLive, height, forks } = data

  return (
    <div className="tree-trunk" style={{ height }} aria-hidden="true">
      <span
        className={[
          'tree-trunk-capsule',
          isAuthorityPath ? 'trunk-authority' : 'trunk-candidate',
          isAbandoned(branch.status) ? 'trunk-abandoned' : '',
          isLive ? 'trunk-live' : '',
        ]
          .filter(Boolean)
          .join(' ')}
      />
      {forks.map((fork) => (
        <Handle
          key={fork.childId}
          type="source"
          position={Position.Right}
          id={`fork:${fork.childId}`}
          className="tree-handle tree-fork-handle"
          style={{ top: fork.offset }}
        />
      ))}
    </div>
  )
}

export const BranchTrunk = memo(BranchTrunkView)
