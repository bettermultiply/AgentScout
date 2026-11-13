import { useEffect, useState } from 'react'
import {
  DEFAULT_TUNING,
  hasTuningParams,
  normalizeTuning,
  tuningFromSearch,
  tuningToSearch,
  type TreeTuning,
} from './tree'

const STORAGE_KEY = 'ascout-tree-tuning'

function initiallyTuned(): TreeTuning {
  if (hasTuningParams(window.location.search)) return tuningFromSearch(window.location.search)
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY)
    if (stored) return normalizeTuning(JSON.parse(stored) as Partial<TreeTuning>)
  } catch {
    // A corrupt stored value falls back to the defaults.
  }
  return DEFAULT_TUNING
}

/**
 * Drawing parameters for the tree. URL values win over the last stored values,
 * so a copied link reproduces a shape even in another browser.
 */
export function useTreeTuning(): [TreeTuning, (next: TreeTuning) => void] {
  const [tuning, setTuning] = useState<TreeTuning>(initiallyTuned)

  useEffect(() => {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(tuning))
    } catch {
      // Storage may be unavailable; the URL still carries the values.
    }
    const url = new URL(window.location.href)
    url.search = tuningToSearch(tuning, url.search)
    window.history.replaceState(null, '', url)
  }, [tuning])

  return [tuning, setTuning]
}
