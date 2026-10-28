import { useCallback, useState } from 'react'
import type { AppSummary } from './api/apps'
import { AppDetail } from './components/AppDetail'
import { AppGrid } from './components/AppGrid'
import { Toolbar } from './components/Toolbar'
import { useApps } from './hooks/useApps'

function App() {
  const { apps, loading, error, install, start, stop, refresh, dismissError } =
    useApps()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const closeDetail = useCallback(() => setSelectedId(null), [])

  function handleAction(app: AppSummary) {
    if (app.state === 'not_installed') {
      void install(app.id)
    } else if (app.state === 'stopped') {
      void start(app.id)
    } else if (app.state === 'running' && app.managed) {
      void stop(app.id)
    }
  }

  return (
    <div className="app">
      <Toolbar onImported={() => void refresh()} />
      {error && (
        <div className="banner-error" role="alert">
          <span>{error}</span>
          <button
            type="button"
            className="banner-retry"
            onClick={() => {
              dismissError()
              void refresh()
            }}
          >
            Retry
          </button>
        </div>
      )}
      {loading ? (
        <main className="grid-viewport">
          <p className="grid-message">Loading applications…</p>
        </main>
      ) : (
        <AppGrid items={apps} onOpen={setSelectedId} onAction={handleAction} />
      )}
      {selectedId && (
        <AppDetail
          appId={selectedId}
          onClose={closeDetail}
          onCatalogChanged={() => void refresh()}
        />
      )}
    </div>
  )
}

export default App