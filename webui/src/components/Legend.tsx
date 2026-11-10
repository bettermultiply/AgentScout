import { ReasonIcon, StatusIcon } from '../icons'

export function Legend() {
  return (
    <details className="legend">
      <summary>图例</summary>
      <div className="legend-body">
        <section>
          <h4>投机原因</h4>
          <p className="legend-note">
            来自 <code>branch_created.speculation_kind</code>；未枚举的值归为“未知/其他”。
          </p>
          <ul>
            <li>
              <span className="legend-icon reason-tool">
                <ReasonIcon reason="tool" />
              </span>
              工具执行
            </li>
            <li>
              <span className="legend-icon reason-model">
                <ReasonIcon reason="model" />
              </span>
              模型推理
            </li>
            <li>
              <span className="legend-icon reason-other">
                <ReasonIcon reason="other" />
              </span>
              未知/其他
            </li>
          </ul>
        </section>
        <section>
          <h4>节点与连线</h4>
          <ul>
            <li>
              <span className="legend-node legend-authority" aria-hidden="true">
                <StatusIcon status="promoted" />
              </span>
              权威路径：实色，连线加粗
            </li>
            <li>
              <span className="legend-node legend-candidate" aria-hidden="true">
                <StatusIcon status="running" />
              </span>
              投机候选：同色系空心描边，运行中脉动
            </li>
            <li>
              <span className="legend-node legend-discarded" aria-hidden="true">
                <StatusIcon status="discarded" />
              </span>
              已抛弃：灰色降透明度
            </li>
          </ul>
        </section>
      </div>
    </details>
  )
}
