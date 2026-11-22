import { Component, type ErrorInfo, type ReactNode } from 'react'

interface Props {
  children: ReactNode
}

interface State {
  error: Error | null
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error('AScout WebUI render error', error, info)
  }

  render() {
    if (this.state.error) {
      return (
        <div className="fatal">
          <h1>界面渲染失败</h1>
          <p>观察接口仍然可用；刷新页面可重新取得状态事件集。</p>
          <pre>{this.state.error.message}</pre>
          <button type="button" onClick={() => window.location.reload()}>
            刷新页面
          </button>
        </div>
      )
    }
    return this.props.children
  }
}
