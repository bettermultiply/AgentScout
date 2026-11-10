import { ReasonIcon, StatusIcon } from '../icons'
import { leafKindLabel, type LeafKind, type LeafTone } from '../treeLayout'

const leafSamples: Array<{ kind: LeafKind; tone: LeafTone; note: string }> = [
  { kind: 'conversation', tone: 'neutral', note: '流式回复逐块上报；连续更新合并' },
  { kind: 'model', tone: 'accent', note: '一次模型调用的 token 与费用' },
  { kind: 'tool', tone: 'success', note: '成功为绿色，失败为红色' },
  { kind: 'status', tone: 'warning', note: '生命周期状态变化' },
  { kind: 'environment', tone: 'info', note: '独立环境或清理结果' },
  { kind: 'settlement', tone: 'success', note: '命中；未命中为红色' },
  { kind: 'authority', tone: 'accent', note: '权威窗口切换' },
]

export function Legend() {
  return (
    <details className="legend">
      <summary>图例</summary>
      <div className="legend-body">
        <section>
          <h4>树枝</h4>
          <p className="legend-note">
            一个 Branch 是一根纵向树枝；纵向只表示事件顺序，长度不代表时间。分支名不显示。
          </p>
          <ul>
            <li>
              <span className="legend-trunk legend-trunk-authority" aria-hidden="true" />
              权威树枝：实色，更粗
            </li>
            <li>
              <span className="legend-trunk legend-trunk-candidate" aria-hidden="true" />
              投机候选：同色系空心描边，运行中轻微脉动
            </li>
            <li>
              <span className="legend-trunk legend-trunk-abandoned" aria-hidden="true" />
              已抛弃：灰色降透明度
            </li>
            <li>
              <span className="legend-head" aria-hidden="true">
                <StatusIcon status="running" size={18} />
                <span className="legend-head-reason">
                  <ReasonIcon reason="tool" size={11} />
                </span>
              </span>
              树枝头部：状态图标 + 投机原因；点击进入分支详情
            </li>
          </ul>
        </section>
        <section>
          <h4>叶子（Branch 内的事件）</h4>
          <ul>
            {leafSamples.map((sample) => (
              <li key={sample.kind}>
                <span
                  className={`legend-leaf tree-leaf leaf-${sample.kind} tone-${sample.tone}`}
                  aria-hidden="true"
                >
                  <span className="leaf-shape" />
                </span>
                <span>
                  {leafKindLabel(sample.kind)}
                  <span className="legend-leaf-note">{sample.note}</span>
                </span>
              </li>
            ))}
          </ul>
          <p className="legend-note">
            连续且呈现信息相同的同类事件合并为一个叶子；悬停显示条数与详情，点击叶子进入该分支详情。
          </p>
        </section>
        <section>
          <h4>投机阶段与原因</h4>
          <p className="legend-note">
            Chance 阶段来自 <code>chance_stage_changed</code>，抛弃原因和阻塞原因分别来自
            <code>ChanceStage::Discarded</code> 与 <code>BranchStatus::Blocked</code>。
          </p>
          <ul>
            <li>
              <code>pending</code> 待选 · <code>running</code> 运行中 ·{' '}
              <code>resolving</code> 核实中
            </li>
            <li>
              <code>accepted</code> 已接受某个 Proposal；<code>discarded</code>{' '}
              已抛弃
            </li>
          </ul>
          <p className="legend-note">抛弃原因</p>
          <ul>
            <li>
              <code>match_rejected</code> 匹配失败 · <code>not_selected</code> 未被选中
            </li>
            <li>
              <code>ancestor_invalidated</code> 祖先失效 ·{' '}
              <code>budget_exhausted</code> 预算耗尽 · <code>failed</code> 执行失败
            </li>
          </ul>
          <p className="legend-note">阻塞原因</p>
          <ul>
            <li>
              <code>extent_reached</code> 达到 extent · <code>rebalanced</code> 被
              rebalance 暂停 · <code>effect_gate</code> effect 未放行
            </li>
          </ul>
        </section>
      </div>
    </details>
  )
}
