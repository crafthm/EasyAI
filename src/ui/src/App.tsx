import { useCallback, useState } from 'react'
import type { AppSummary } from './api/apps'
import { AppDetail } from './components/AppDetail'
import { AppGrid } from './components/AppGrid'
import { InstallPlanDialog } from './components/InstallPlanDialog'
import { Toolbar } from './components/Toolbar'
import { useApps } from './hooks/useApps'

function App() {
  const { apps, loading, error, install, start, stop, refresh, dismissError } =
    useApps()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [planApp, setPlanApp] = useState<AppSummary | null>(null)
  const closeDetail = useCallback(() => setSelectedId(null), [])
  const closePlan = useCallback(() => setPlanApp(null), [])

  function handleAction(app: AppSummary) {
    if (app.draft) return
    if (app.state === 'not_installed') {
      setPlanApp(app)
    } else if (app.state === 'stopped') {
      void start(app.id)
    } else if (app.state === 'running' && app.managed) {
      void stop(app.id)
    }
  }

  function handlePrimaryAction(appId: string) {
    const app = apps.find((item) => item.id === appId)
    if (app) handleAction(app)
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
        <AppGrid items={apps} onOpen={setSelectedId} />
      )}
      {selectedId && (
        <AppDetail
          appId={selectedId}
          onClose={closeDetail}
          onCatalogChanged={() => void refresh()}
          onPrimaryAction={handlePrimaryAction}
        />
      )}
      {planApp && (
        <InstallPlanDialog
          app={planApp}
          onClose={closePlan}
          onInstalled={() => void refresh()}
          install={install}
        />
      )}
    </div>
  )
}

export default App