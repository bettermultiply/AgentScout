import { createRoot } from 'react-dom/client'
import { SpeculationPanel } from './src/components/SpeculationPanel'
import { useAppStore } from './src/store'
import { DEFAULT_TUNING, treeLayoutStats } from './src/tree'
import './src/styles.css'

// Seconds on a shared clock. Candidate starts and tool timings are hand authored.
// Each turn is [model start, model duration, tools]; a tool may open a Chance.
const dense = ['主枝', [
  [0.7, 2.8, [['查看目录', 3.9, 5.2, [
    ['找入口', [
      [4.8, 3.3, [['搜索入口', 8.4, 5.6, [
        ['追踪路由', [[9.3, 4.1, [['读路由', 14, 1.3]]], [16.2, 2.3, [['核对调用方', 19, 4.3, [
          ['核对初始化', [[20.1, 2.7, [['读初始化', 23.4, 1.8]]]]],
          ['核对配置', [[21.4, 3.8, [['读配置', 25.7, 2.4]]]]],
        ]]]]]],
        ['追踪命令', [[10.5, 4.7, [['读命令', 15.8, 1.5]]], [18.7, 2.2, []]]],
      ]], ['读项目说明', 8.7, 1.9]]],
      [15.4, 2.9, [['读入口', 18.8, 2.6]]],
    ]],
    ['找配置', [[5.6, 4.4, [['搜索配置', 10.4, 3.6]]], [15, 2.4, [['读默认值', 17.9, 1.6]]]]],
  ]], ['查看工作区', 4.1, 0.9]]],
  [10, 3.5, [['读配置定义', 14.1, 4.8, [
    ['核对环境变量', [[15.2, 3.9, [['搜索变量', 19.6, 5.3, [
      ['核对覆盖顺序', [[20.6, 4.2, [['读加载顺序', 25.4, 2.8]]], [29.3, 2.5, []]]],
    ]]]]]],
  ]]]],
  [20, 3.2, [['搜索调用方', 23.8, 6.2, [
    ['检查直接调用', [[24.7, 3.6, [['读调用链', 28.8, 2.1], ['找关联样例', 29.2, 1.2]]], [32.1, 3.4, []]]],
    ['检查间接调用', [[25.9, 4.6, [['搜索封装层', 31.1, 5.8, [
      ['检查兼容层', [[32.4, 3.7, [['读兼容层', 36.7, 2.3]]], [40.1, 3.1, []]]],
      ['追踪旧入口', [[33.6, 5.1, [['搜索旧入口', 39.3, 2.9, null, 'failed']]]]],
    ]]]]]],
  ]]]],
  [31.3, 5.2, [['读测试样例', 37, 7.4, [
    ['检查边界输入', [[38.2, 3.3, [['读异常样例', 42, 2]]], [45.3, 2.6, [['核对断言', 48.4, 4.9, [
      ['对照空输入', [[49.5, 3.5, [['读空输入样例', 53.5, 1.7]]]]],
    ]]]]]],
    ['检查正常输入', [[39.5, 4.5, [['读正常样例', 44.6, 2.5]]], [48.3, 3.1, []]]],
    ['检查历史样例', [[41.4, 2.7, [['搜索历史样例', 44.8, 3.8]]]]],
  ]]]],
  [45.7, 4.3, [['核对输出格式', 50.6, 6.9, [
    ['对照文档', [[51.7, 3.6, [['读格式说明', 55.8, 2.6]]], [59.5, 3.2, []]]],
    ['对照序列化', [[52.9, 4.8, [['读序列化代码', 58.2, 6.5, [
      ['检查可选字段', [[59.6, 3.3, [['读字段声明', 63.5, 2.2]]]]],
      ['检查版本字段', [[61.2, 2.9, [['读版本约定', 64.8, 1.4]]]]],
    ]]]]]],
  ]]]],
  [58.6, 5.5, [['核对回归用例', 64.6, 7.7, [
    ['检查回归记录', [[65.9, 4.4, [['读回归记录', 70.9, 2.7]]], [74.7, 3.6, []]]],
  ]]]],
  [73.5, 6.1, []],
]]

const alternate = ['任务入口', [
  [0.5, 3, [['准备输入', 4, 3, [
    ['先行分支', [
      [4.8, 2.4, [['比对结果', 8, 3, [
        ['无调用候选', []],
        ['追加处理', [[9.1, 4, [['读取结果', 13.5, 2]]], [21, 2.5, []], [32, 2, []]]],
      ]]]],
      [30, 4, [['返回结果', 35, 1, null, 'cancelled']]],
    ]],
  ]]]],
  [12, 3.8, [['等待准入', 17, 2, []]]],
  [23, 4.3, [['后续处理', 28, 2, [
    ['并行分支', [[29, 2.5, [['提取内容', 32, 2, null, 'timeout']]], [39, 4, []]]],
    ['备选', []],
  ]]]],
  [38, 3.1, []],
]]

const ms = seconds => Math.round(seconds * 1000)
function makeEvents(taskId, root) {
  const events = []
  function record(timestamp, event, fields) {
    events.push({ event, task_id: taskId, timestamp_ms: ms(timestamp), ...fields })
  }
  function addBranch([branch_id, turns], born, parent_branch_id = null, chance_id = null, opened = null) {
    record(born, 'branch_created', {
      branch_id, parent_branch_id, chance_id, role: parent_branch_id ? 'candidate' : 'authority',
      speculation_kind: parent_branch_id ? 'tool_execution' : null,
      speculation_started_at_ms: opened === null ? null : ms(opened),
    })
    for (const [start, duration, tools] of turns) {
      const callTiming = (at, elapsed) => ({
        started_at_ms: ms(at), duration_ms: ms(elapsed), finished_at_ms: ms(at + elapsed),
      })
      record(start + duration, 'model_usage', {
        branch_id, ...callTiming(start, duration), model: '示例模型', action_id: `${branch_id}:模型:${start}`,
        cost: null, input_tokens: null, output_tokens: null, total_tokens: null,
        cache_read_tokens: null, cache_write_tokens: null,
      })
      for (const [tool_name, at, elapsed, candidates, status = 'succeeded'] of tools) {
        const action_id = `${branch_id}:${tool_name}`
        record(at + elapsed, 'tool_finished', { branch_id, tool_name, action_id, status, ...callTiming(at, elapsed) })
        if (!candidates) continue
        const nestedChanceId = `机会·${tool_name}`
        record(at + 0.3, 'chance_created', {
          chance_id: nestedChanceId, branch_id, parent_branch_id: branch_id, parent_chance_id: chance_id,
          operation_id: action_id, tool_name, stage: 'running', authority_state: 'fetching',
          speculation_started_at_ms: ms(at),
        })
        for (const candidate of candidates) {
          const born = Math.max(at + 0.31, (candidate[1][0]?.[0] ?? at + 1) - 0.2)
          addBranch(candidate, born, branch_id, nestedChanceId, at)
        }
        record(at + elapsed + 0.07, 'authority_state_changed', { chance_id: nestedChanceId, state: 'arrived' })
      }
    }
  }
  addBranch(root, 0)
  return events.sort((a, b) => a.timestamp_ms - b.timestamp_ms)
}

const denseTask = '密集 12 Chance'
const alternateTask = '单候选、空芽与空 Chance'
const events = [...makeEvents(denseTask, dense), ...makeEvents(alternateTask, alternate)]
const store = useAppStore.getState()
store.setTreeTuning({ ...DEFAULT_TUNING })
events.forEach((event, index) => store.ingestEvent({ ...event, sequence: index + 1 }))
store.selectTask(denseTask)

// Branch details can read the same offline fixtures without a backend.
window.fetch = async input => {
  const match = typeof input === 'string' && input.match(/^\/api\/branches\/([^/]+)\/conversation$/)
  if (!match) throw new Error('离线演示没有此接口')
  const state = useAppStore.getState()
  const branch = state.state.tasks[state.selectedTaskId].branches[decodeURIComponent(match[1])]
  return Response.json({ inherited: [], messages: [], streaming: '', role: branch.role, status: branch.status })
}

window.treeReview = { useAppStore, treeLayoutStats }

function Review() {
  const taskId = useAppStore(state => state.selectedTaskId)
  return <>
    <header className="review-toolbar">
      <nav aria-label="示例">
        {[denseTask, alternateTask].map(id => <button type="button" key={id}
          aria-pressed={id === taskId} onClick={() => { store.showTree(); store.selectTask(id) }}>{id}</button>)}
      </nav>
      <span>悬停枝干、叶茎、叶片或芽。拖动平移，＋／− 缩放。</span>
    </header>
    <SpeculationPanel />
  </>
}

createRoot(document.querySelector('#review')).render(<Review />)
