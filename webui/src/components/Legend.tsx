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
          <h4>大枝（一个 Branch）</h4>
          <p className="legend-note">
            初始 Branch 保持直立；子枝从父枝 Chance 的逻辑时间位置长出，各自按稳定方向逐渐回正。权威切换只改变样式，不移动枝干。枝长取该 Branch 从创建到最后一个事件的逻辑时间；枝粗取该点之后（含子枝）还剩多少调用。分支名不显示。
          </p>
          <ul>
            <li>
              <span className="legend-trunk legend-trunk-authority" aria-hidden="true" />
              当前权威：近白（浅色主题近黑）实色，最粗
            </li>
            <li>
              <span className="legend-trunk legend-trunk-finished" aria-hidden="true" />
              历史权威：同色略淡，主干从种子到当前始终连着
            </li>
            <li>
              <span className="legend-trunk legend-trunk-candidate" aria-hidden="true" />
              投机候选：粉紫，细于父枝
            </li>
            <li>
              <span className="legend-trunk legend-trunk-abandoned" aria-hidden="true" />
              未采纳：灰色、整枝约 40% 透明，叶片变枯
            </li>
            <li>
              <span className="legend-head" aria-hidden="true">
                <StatusIcon status="running" size={10} />
                <span className="legend-head-reason">
                  <ReasonIcon reason="tool" size={6} />
                </span>
              </span>
              主干起点：点击进入详情顶部；子枝不直接长在父干上，不画根盘
            </li>
          </ul>
        </section>
        <section>
          <h4>模型叶茎</h4>
          <ul>
            <li>
              <TwigSample />
              每次模型调用一根短弧形叶茎，长度取该次调用耗时；无工具时末端为小模型标记
            </li>
          </ul>
          <p className="legend-note">
            叶茎从枝的轮廓上直接长出，不悬空；工具叶片沿叶茎方向排列，叶片大小取工具耗时。下一次模型调用开始新的叶茎，不合并模型调用。悬停显示模型名、token、耗时与费用；缺失字段显示“未上报”。点击定位到详情中的这次调用。
          </p>
        </section>
        <section>
          <h4>工具叶片</h4>
          <ul>
            <li>
              <span className="legend-leaf tone-success" aria-hidden="true">
                <span className="leaf-shape" />
              </span>
               成功：权威枝上实心绿，候选枝上实心玫红
            </li>
            <li>
              <span className="legend-leaf tone-danger" aria-hidden="true">
                <span className="leaf-shape" />
              </span>
               失败：实心红
            </li>
            <li>
              <span className="legend-leaf tone-cancelled" aria-hidden="true">
                <span className="leaf-shape leaf-hollow" />
              </span>
               取消：空心灰，半透明
            </li>
            <li>
              <span className="legend-leaf tone-timeout" aria-hidden="true">
                <span className="leaf-shape leaf-hollow" />
              </span>
               超时：空心琥珀
            </li>
          </ul>
          <p className="legend-note">
            工具叶片长在弯曲叶茎上，沿叶茎方向排列；第一次模型调用之前的工具使用相同轮廓。候选 Branch 上的成功叶取枝的玫红，失败、取消、超时仍用红、空心灰、空心琥珀，整枝不再出现绿枝粉叶的割裂。悬停显示工具名、状态与耗时，点击定位到详情中的这次调用。枝未采纳时所有叶片一起变枯，不再单独显示工具状态。
          </p>
        </section>
        <section>
          <h4>结算标记</h4>
          <p className="legend-note">
            枝头旁的 ✓ 表示命中，红色 ✕ 表示未命中；悬停枝头查看实测与估计结算，不合并、不补零。
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
               未采纳
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
