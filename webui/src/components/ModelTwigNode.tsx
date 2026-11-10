import { memo } from 'react'
import type { Node, NodeProps } from '@xyflow/react'
import type { BranchState } from '../fold'
import {
  TWIG_HEIGHT,
  TWIG_WIDTH,
  type CallGroup,
} from '../treeLayout'
import { useAppStore } from '../store'

export interface ModelTwigData extends Record<string, unknown> {
  branch: BranchState
  group: CallGroup
  isAuthorityPath: boolean
  direction: -1 | 1
}

export type ModelTwigFlowNode = Node<ModelTwigData, 'modelTwig'>

export const MODEL_TWIG_WIDTH = TWIG_WIDTH
export const MODEL_TWIG_HEIGHT = TWIG_HEIGHT

function isAbandoned(status: BranchState['status']): boolean {
  return status === 'discarded' || status === 'cleaned'
}

function ModelTwigView({ data }: NodeProps<ModelTwigFlowNode>) {
  const { branch, group, direction } = data
  const open = () =>
    useAppStore.getState().openBranch(branch.branchId, {
      kind: 'model',
      actionId: group.events[0]?.action_id ?? null,
      sequences: group.events.map((event) => event.sequence),
    })
  const count = group.events.length > 1 ? `，连续 ${group.events.length} 条` : ''
  const models = [...new Set(
    group.events.map((event) => (event.event === 'model_usage' ? event.model : '')),
  )].join('、')

  return (
    <div
      className={[
        'tree-twig',
        direction > 0 ? 'twig-right' : 'twig-left',
        `tone-${group.tone}`,
        isAbandoned(branch.status) ? 'twig-abandoned' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      tabIndex={0}
      role="button"
      aria-label={`模型调用：${models}，事件 ${group.firstSequence}${
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
      <span className="twig-line" aria-hidden="true" />
      <span className="twig-leaf" aria-hidden="true" />
      {group.events.length > 1 ? (
        <span className="twig-count" aria-hidden="true">
          ×{group.events.length}
        </span>
      ) : null}
    </div>
  )
}

export const ModelTwigNode = memo(ModelTwigView)
