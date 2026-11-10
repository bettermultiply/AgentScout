const numberFormat = new Intl.NumberFormat('zh-CN')

export function formatCount(value: number): string {
  return numberFormat.format(value)
}

export function formatTokens(value: number): string {
  return numberFormat.format(Math.round(value))
}

export function formatCost(value: number): string {
  if (value === 0) return '$0.0000'
  const digits = Math.abs(value) >= 1 ? 2 : 4
  return `$${value.toLocaleString('en-US', {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  })}`
}

export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms)) return '未上报'
  const absolute = Math.abs(ms)
  if (absolute < 1000) return `${Math.round(ms)} ms`
  if (absolute < 60_000) {
    const seconds = ms / 1000
    return `${seconds.toFixed(seconds < 10 ? 1 : 0)} s`
  }
  const totalSeconds = Math.round(ms / 1000)
  const minutes = Math.trunc(totalSeconds / 60)
  const seconds = Math.abs(totalSeconds % 60)
  if (Math.abs(minutes) < 60) return `${minutes} 分 ${seconds} 秒`
  const hours = Math.trunc(minutes / 60)
  return `${hours} 小时 ${Math.abs(minutes % 60)} 分`
}

export function formatSignedDuration(ms: number): string {
  const text = formatDuration(Math.abs(ms))
  return `${ms >= 0 ? '+' : '−'}${text}`
}

export function formatMoney(value: number, currency: string): string {
  if (!currency || currency === '未上报') {
    return `${value.toLocaleString('en-US', { maximumFractionDigits: 4 })}（币种未上报）`
  }
  try {
    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 4,
    }).format(value)
  } catch {
    return `${value.toLocaleString('en-US', { maximumFractionDigits: 4 })} ${currency}`
  }
}

export function formatSignedMoney(value: number, currency: string): string {
  return `${value >= 0 ? '+' : '−'}${formatMoney(Math.abs(value), currency)}`
}

export function formatTimestamp(ms: number | null): string {
  if (ms === null) return '未上报'
  const date = new Date(ms)
  const now = new Date()
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate()
  const time = date.toLocaleTimeString('zh-CN', { hour12: false })
  if (sameDay) return time
  return `${date.toLocaleDateString('zh-CN')} ${time}`
}

export function shortId(id: string): string {
  return id.length <= 18 ? id : `${id.slice(0, 8)}…${id.slice(-6)}`
}

export function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}
