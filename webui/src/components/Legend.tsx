import { ReasonIcon, StatusIcon } from '../icons'

function TwigSample() {
  return (
    <span className="legend-twig" aria-hidden="true">
      <span className="legend-twig-line" />
      <span className="legend-twig-leaf" />
    </span>
  )
}

export function Legend() {
  return (
    <details className="legend">
      <summary>图例</summary>
      <div className="legend-body">
        <section>
          <h4>树枝（一个 Branch）</h4>
          <p className="legend-note">
            权威路径是居中的主干；投机候选从主干或父候选分叉，按子树大小分配到左右两侧斜向下生长。纵向只表示调用顺序，长度不代表时间；分支名不显示。
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
              树枝头部：生命周期状态 + 投机原因；点击进入详情顶部
            </li>
            <li>
              <span
                className="legend-settlement legend-settlement-hit"
                aria-hidden="true"
              >
                ✓
              </span>
              头部旁的结算标记：命中；未命中为红色 ✕
            </li>
          </ul>
        </section>
        <section>
          <h4>小枝条（模型调用）</h4>
          <ul>
            <li>
              <TwigSample />
              每次模型调用一根小枝条，末端小叶表示结果；比叶子大、比树枝细
            </li>
          </ul>
          <p className="legend-note">
            悬停显示模型名、token、耗时与费用；缺失字段显示“未上报”。连续同模型调用合并为一条并带计数，悬停展开完整列表。点击定位到详情中的这次调用。
          </p>
        </section>
        <section>
          <h4>叶子（工具调用）</h4>
          <ul>
            <li>
              <span
                className="legend-leaf tree-leaf leaf-tool tone-success"
                aria-hidden="true"
              >
                <span className="leaf-shape" />
              </span>
              成功：绿色方形叶子
            </li>
            <li>
              <span
                className="legend-leaf tree-leaf leaf-tool tone-danger"
                aria-hidden="true"
              >
                <span className="leaf-shape" />
              </span>
              失败：红色；取消为灰色，超时为黄色
            </li>
          </ul>
          <p className="legend-note">
            沿树枝的每次工具调用是一片叶子，悬停显示工具名、状态与耗时。连续同名且同结果的调用合并为一片并带计数，悬停展开完整列表。点击定位到详情中的这次调用。
          </p>
        </section>
        <section>
          <h4>投机原因与 Chance</h4>
          <p className="legend-note">
            头部原因徽标来自 <code>branch_created.speculation_kind</code>；Chance
            字段来自 <code>chance_created</code> 与后续阶段事件。
          </p>
          <ul>
            <li>
              <ReasonIcon reason="tool" size={13} /> 工具执行 ·{' '}
              <ReasonIcon reason="model" size={13} /> 模型推理 ·{' '}
              <ReasonIcon reason="other" size={13} /> 其他 / 未上报
            </li>
            <li>
              <code>chance_id</code> 区分这是哪一次投机；初始权威为 null
            </li>
            <li>
              <code>chance_created.tool_name</code> 是发起投机的工具名；缺失显示“未上报”
            </li>
            <li>
              <code>parent_chance_id</code> 表示嵌套投机；null 显示“未上报”
            </li>
          </ul>
          <p className="legend-note">Chance 阶段</p>
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
          <p className="legend-note">权威状态</p>
          <ul>
            <li>
              <code>not_requested</code> 未请求 · <code>fetching</code> 正在取得 ·{' '}
              <code>arrived</code> 已到达 · <code>unavailable</code> 不可用
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
