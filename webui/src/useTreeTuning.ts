import { useEffect } from 'react'
import { useAppStore } from './store'
import {
  removeTuningParams,
  type TreeTuning,
} from './tree'

const STORAGE_KEY = 'ascout-tree-tuning'

export function useTreeTuning(): [TreeTuning, (next: TreeTuning) => void] {
  const tuning = useAppStore(store => store.treeTuning)
  const setTuning = useAppStore(store => store.setTreeTuning)

  useEffect(() => {
    const url = new URL(window.location.href)
    if (removeTuningParams(url.searchParams)) {
      window.history.replaceState(window.history.state, '', url)
    }
  }, [])

  useEffect(() => {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(tuning))
    } catch {
    }
  }, [tuning])

  return [tuning, setTuning]
}
