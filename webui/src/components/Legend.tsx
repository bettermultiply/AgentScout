import { ReasonIcon } from '../icons'

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
            根枝初始直立。子枝从源侧准入时间对应的父枝中心线长出。枝长取相邻调用的开始时间差，枝粗取该位置之后（含子枝）的剩余调用量。悬停枝干、叶茎、叶片或芽时，该枝和后代会移到前景。点击枝干或芽可进入详情。
          </p>
          <ul>
            <li>
              <span className="legend-trunk legend-trunk-authority" aria-hidden="true" />
              当前权威为暖灰色，深色主题下为浅暖灰
            </li>
            <li>
              <span className="legend-trunk legend-trunk-finished" aria-hidden="true" />
              历史权威沿用暖灰色，颜色更淡
            </li>
            <li>
              <span className="legend-trunk legend-trunk-candidate" aria-hidden="true" />
              投机候选为灰绿色
            </li>
            <li>
              <span className="legend-trunk legend-trunk-abandoned" aria-hidden="true" />
              未采纳枝及其叶片为灰色，枝与叶保留 40% 不透明度
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
              成功为实心，权威枝上为暖灰，候选枝上为灰绿
            </li>
            <li>
              <span className="legend-leaf tone-danger" aria-hidden="true">
                <span className="leaf-shape" />
              </span>
               失败为实心暖红
            </li>
            <li>
              <span className="legend-leaf tone-cancelled" aria-hidden="true">
                <span className="leaf-shape leaf-hollow" />
              </span>
               取消为空心灰
            </li>
            <li>
              <span className="legend-leaf tone-timeout" aria-hidden="true">
                <span className="leaf-shape leaf-hollow" />
              </span>
               超时为空心琥珀色
            </li>
          </ul>
          <p className="legend-note">
            尖叶沿弯曲的叶茎排列。悬停显示工具名、状态与耗时，点击定位到详情中的这次调用。枝未采纳时叶片统一为灰色，取消和超时仍为空心。
          </p>
        </section>
        <section>
          <h4>投机原因与 Chance</h4>
          <p className="legend-note">
            <span className="legend-chance" aria-hidden="true" /> 圆环标出 Chance，可悬停查看，有且仅有一个候选时可点击进入该候选详情。投机原因来自 <code>branch_created.speculation_kind</code>。
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
