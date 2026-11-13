// All shape controls, including fixed/range modes, live in this file.
const EXTRA_DEFAULTS = {
  stemTauMs: 1000, leafTauMs: 1000, leafReferenceMs: 12000,
  leafWidthRatio: .45, leafWidthRandom: 0, leafWidthMin: .3, leafWidthMax: .6,
  stemAngle: 20, stemAngleRandom: 0, stemAngleMin: 10, stemAngleMax: 30,
  stemBend: 1, stemBendRandom: 0, stemBendMin: .5, stemBendMax: 1.5,
  childAngleMin: 30, childAngleMax: 90,
  leafStart: .45, leafSpread: 35, leafSideProbability: .7,
  budLength: 30, budWidth: 4.5, modelTipRadius: 1.5, chanceRadiusRatio: .05,
  nodeOrigin: 64, sampleGap: 4, stemOppositeBias: .5,
  petioleLength: 6, petioleWidth: 1.4, leafStrokeWidth: 1, veinWidth: 1,
  leafOpacity: 1, veinStart: .1, veinEnd: .85, stemAnchor: .5,
  stemControlX: .5, stemControlY: .1, stemTipY: .14,
  connectionParentRatio: .15, connectionRootRatio: .15,
  startLeadRatio: .5,
}
type ExtraTuning = typeof EXTRA_DEFAULTS
export interface TuneField { key: keyof TreeTuning; label: string; min: number; max: number; step: number; scale?: number; group: string; mode?: boolean; visibleWhen?: { key: keyof TreeTuning; value: number } }
const field = (key: keyof ExtraTuning, label: string, min: number, max: number, step: number, group: string, scale?: number): TuneField => ({ key, label, min, max, step, group, scale })
export const EXTRA_FIELDS: TuneField[] = [
  field('childAngleMin', '子枝夹角下限（°）', 5, 90, 1, '树枝'),
  field('childAngleMax', '子枝夹角上限（°）', 5, 90, 1, '树枝'),
  field('startLeadRatio', '起点提前量 / 本枝根宽', 0, 3, .05, '树枝'),
  field('connectionParentRatio', '连接父侧距离 / 子枝根宽', 0, 2, .01, '树枝'),
  field('connectionRootRatio', '连接子侧距离 / 子枝根宽', 0, 2, .01, '树枝'),
  field('stemTauMs', '叶茎时间尺度（毫秒）', 1, 300000, 100, '叶茎'),
  field('leafTauMs', '叶片时间尺度（毫秒）', 1, 300000, 100, '叶片'),
  field('leafReferenceMs', '叶长参考时长（毫秒）', 1, 300000, 100, '叶片'),
  ...(['leafWidth', 'stemAngle', 'stemBend'] as const).flatMap((prefix) => {
    const labels = { leafWidth: '叶宽比例', stemAngle: '叶茎倾角（°）', stemBend: '叶茎上翘倍数' }
    const group = prefix === 'leafWidth' ? '叶片' : '叶茎'
    const max = prefix.includes('Angle') ? 180 : 3
    const step = prefix.includes('Angle') ? 1 : .05
    const modeKey = `${prefix}Random` as const
    return [
      { ...field(modeKey, `${labels[prefix]}模式`, 0, 1, 1, group), mode: true },
      { ...field(prefix === 'leafWidth' ? 'leafWidthRatio' : prefix, labels[prefix], 0, max, step, group), visibleWhen: { key: modeKey, value: 0 } },
      { ...field(`${prefix}Min`, `${labels[prefix]}随机下限`, 0, max, step, group), visibleWhen: { key: modeKey, value: 1 } },
      { ...field(`${prefix}Max`, `${labels[prefix]}随机上限`, 0, max, step, group), visibleWhen: { key: modeKey, value: 1 } },
    ]
  }),
  field('leafStart', '叶在茎上的起点比例', 0, 1, .01, '叶片'),
  field('leafSpread', '叶片散开角（°）', 0, 180, 1, '叶片'),
  field('leafSideProbability', '叶片反侧概率', 0, 1, .01, '叶片'),
  field('budLength', '空枝芽长度', 0, 200, 1, '树枝'), field('budWidth', '空枝芽宽度', 0, 100, .5, '树枝'),
  field('modelTipRadius', '模型端点半径', 0, 30, .1, '叶茎'),
  field('chanceRadiusRatio', 'Chance 半径 / 子枝根宽', 0, 2, .05, '树枝'),
  field('nodeOrigin', '节点局部原点 X（仅坐标参考）', 1, 500, 1, '整体'),
  field('sampleGap', '树枝曲线采样步长', 1, 30, 1, '树枝'),
  field('stemOppositeBias', '叶茎反向生长偏好', 0, .5, .05, '叶茎'),
  field('petioleLength', '叶柄长度', 0, 50, .5, '叶片'), field('petioleWidth', '叶柄线宽', 0, 12, .1, '叶片'),
  field('leafStrokeWidth', '叶片描边宽度', 0, 12, .1, '叶片'), field('veinWidth', '叶脉线宽', 0, 12, .1, '叶片'),
  field('leafOpacity', '叶片透明度', 0, 1, .05, '叶片'),
  field('veinStart', '叶脉起点比例', 0, 1, .01, '叶片'), field('veinEnd', '叶脉终点比例', 0, 1, .01, '叶片'),
  field('stemAnchor', '叶茎起点 / 枝宽', 0, 1, .05, '叶茎'),
  field('stemControlX', '叶茎控制点横向比例', 0, 1, .05, '叶茎'),
  field('stemControlY', '叶茎控制点上翘比例', 0, 1, .01, '叶茎'),
  field('stemTipY', '叶茎末端上翘比例', 0, 1, .01, '叶茎'),
]

export const TUNE_FIELDS: TuneField[] = [
  { key:'segmentK', label:'段长系数 K', min:5, max:1000, step:1, group:'树枝' },
  { key:'segmentTauMs', label:'时间尺度 τ（秒）', min:.5, max:600, step:1, scale:1000, group:'树枝' },
  { key:'turnStepDeg', label:'每截转角（°）', min:0, max:30, step:.5, group:'树枝' },
  { key:'recenterRange', label:'回中范围', min:5, max:2000, step:5, group:'树枝' },
  { key:'maxLeanDeg', label:'最大倾角（°）', min:5, max:89, step:1, group:'树枝' },
  { key:'rootWidth', label:'根宽', min:2, max:300, step:1, group:'树枝' },
  { key:'widthExponent', label:'宽度指数', min:.05, max:2, step:.05, group:'树枝' },
  { key:'minTrunkWidth', label:'最小枝宽', min:0, max:20, step:.2, group:'树枝' },
  { key:'stemK', label:'叶茎系数', min:0, max:200, step:.5, group:'叶茎' },
  { key:'stemWidth', label:'叶茎宽度', min:.2, max:12, step:.2, group:'叶茎' },
  { key:'leafMin', label:'叶长最小', min:0, max:200, step:1, group:'叶片' },
  { key:'leafMax', label:'叶长最大', min:0, max:300, step:1, group:'叶片' },
  ...EXTRA_FIELDS,
]

export function stableRandom(value: string): number {
  let h = 2166136261
  for (let i = 0; i < value.length; i++) h = Math.imul(h ^ value.charCodeAt(i), 16777619)
  return (h >>> 0) / 4294967296
}
export function ranged(t: TreeTuning, prefix: 'leafWidth' | 'stemAngle' | 'stemBend', id: string): number {
  if (!t[`${prefix}Random`]) return t[prefix === 'leafWidth' ? 'leafWidthRatio' : prefix]
  const a = t[`${prefix}Min`], b = t[`${prefix}Max`]
  return Math.min(a, b) + Math.abs(b - a) * stableRandom(`${id}/${prefix}`)
}

export function childAngleOf(t: TreeTuning, id: string): number {
  const a = t.childAngleMin, b = t.childAngleMax
  return Math.min(a, b) + Math.abs(b - a) * stableRandom(`${id}/childAngle`)
}

/**
 * Live drawing parameters for the speculation tree. Every value is in tree
 * units, so zoom never changes a proportion.
 */
export interface TreeTuning extends ExtraTuning {
  /** Segment length = segmentK * ln(1 + interval / segmentTauMs). */
  segmentK: number
  segmentTauMs: number
  /** Turn per call interval, in degrees. */
  turnStepDeg: number
  /** Distance over which the walk is pulled back to the centre line. */
  recenterRange: number
  /** A Branch may lean but always keeps a downward component. */
  maxLeanDeg: number
  /** Root width in tree units; independent of the length mapping. */
  rootWidth: number
  /** Width = root width * (remaining share) ^ widthExponent. */
  widthExponent: number
  /** The last call still needs an edge to grow its stem from. */
  minTrunkWidth: number
  /** Stem length = stemK * ln(1 + model seconds). */
  stemK: number
  leafMin: number
  leafMax: number
  /** Stem stroke width in tree units. */
  stemWidth: number
}

export const DEFAULT_TUNING: TreeTuning = {
  ...EXTRA_DEFAULTS,
  segmentK: 60,
  segmentTauMs: 10000,
  turnStepDeg: 18,
  recenterRange: 60,
  maxLeanDeg: 72,
  rootWidth: 9,
  widthExponent: 0.1,
  minTrunkWidth: 1.2,
  stemK: 10,
  leafMin: 10,
  leafMax: 24,
  stemWidth: 2.2,
}

type TuningRange = [min: number, max: number]

export const TUNING_RANGES = Object.fromEntries(TUNE_FIELDS.map(f =>
  [f.key, [f.min * (f.scale ?? 1), f.max * (f.scale ?? 1)]],
)) as Record<keyof TreeTuning, TuningRange>

const TUNING_KEYS: Record<keyof TreeTuning, string> = {
  ...Object.fromEntries(EXTRA_FIELDS.map(f => [f.key, f.key])) as Record<keyof ExtraTuning, string>,
  startLeadRatio: 'lead',
  segmentK: 'k',
  segmentTauMs: 'tau',
  turnStepDeg: 'turn',
  recenterRange: 'recenter',
  maxLeanDeg: 'lean',
  rootWidth: 'rootw',
  widthExponent: 'wexp',
  minTrunkWidth: 'wmin',
  stemK: 'stem',
  leafMin: 'lmin',
  leafMax: 'lmax',
  stemWidth: 'swidth',
}

export function normalizeTuning(input: unknown): TreeTuning {
  const out = { ...DEFAULT_TUNING }
  const source = input !== null && typeof input === 'object' ? input as Record<string, unknown> : {}
  for (const key of Object.keys(out) as Array<keyof TreeTuning>) {
    const [min, max] = TUNING_RANGES[key]
    const raw = source[key]
    const value = typeof raw === 'number' || typeof raw === 'string' && raw.trim() !== '' ? Number(raw) : out[key]
    out[key] = Number.isFinite(value) ? Math.min(max, Math.max(min, value)) : DEFAULT_TUNING[key]
  }
  for (const field of TUNE_FIELDS) if (field.mode) out[field.key] = Math.round(out[field.key])
  if (out.leafMax < out.leafMin) out.leafMax = out.leafMin
  return out
}

export function hasTuningParams(search: string): boolean {
  const params = new URLSearchParams(search)
  return Object.values(TUNING_KEYS).some((key) => params.has(key))
}

export function tuningFromSearch(search: string): TreeTuning {
  const params = new URLSearchParams(search)
  const partial: Partial<TreeTuning> = {}
  for (const key of Object.keys(TUNING_KEYS) as Array<keyof TreeTuning>) {
    const raw = params.get(TUNING_KEYS[key])
    if (raw !== null) {
      const value = Number(raw)
      if (Number.isFinite(value)) partial[key] = value
    }
  }
  return normalizeTuning(partial)
}

export function tuningToSearch(tuning: TreeTuning, base = ''): string {
  const params = new URLSearchParams(base)
  for (const key of Object.keys(TUNING_KEYS) as Array<keyof TreeTuning>) {
    params.set(TUNING_KEYS[key], String(tuning[key]))
  }
  return params.toString()
}
