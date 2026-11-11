import { useCallback, useMemo, useState } from 'react'
import type { AppSummary } from './api/apps'
import { AppDetail } from './components/AppDetail'
import { AppGrid } from './components/AppGrid'
import { InstallPlanDialog } from './components/InstallPlanDialog'
import { Toolbar, type InstallFilter } from './components/Toolbar'
import { useApps } from './hooks/useApps'

function matchesFilter(app: AppSummary, filter: InstallFilter): boolean {
  if (filter === 'installed') return app.state !== 'not_installed'
  if (filter === 'not_installed') return app.state === 'not_installed'
  return true
}

function matchesQuery(app: AppSummary, query: string): boolean {
  const keyword = query.trim().toLowerCase()
  if (!keyword) return true
  return (
    app.name.toLowerCase().includes(keyword) ||
    app.description.toLowerCase().includes(keyword)
  )
}

function App() {
  const { apps, loading, error, install, start, stop, refresh, dismissError } =
    useApps()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [plan, setPlan] = useState<{
    app: AppSummary
    upgrade: boolean
  } | null>(null)
  const planApp = plan?.app ?? null
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<InstallFilter>('all')
  const closeDetail = useCallback(() => setSelectedId(null), [])
  const closePlan = useCallback(() => setPlan(null), [])

  const visibleApps = useMemo(
    () => apps.filter((app) => matchesFilter(app, filter) && matchesQuery(app, query)),
    [apps, filter, query],
  )

  function handleAction(app: AppSummary) {
    if (app.draft) return
    if (app.state === 'not_installed') {
      setPlan({ app, upgrade: false })
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

  // Reinstalling through the plan dialog is the upgrade path: the release
  // method resolves the newest tag, installs it beside the old version, and
  // switches the active directory.
  function handleUpgrade(appId: string) {
    const app = apps.find((item) => item.id === appId)
    if (app) setPlan({ app, upgrade: true })
  }

  return (
    <div className="app">
      <Toolbar
        onImported={() => void refresh()}
        query={query}
        onQueryChange={setQuery}
        filter={filter}
        onFilterChange={setFilter}
      />
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
        <AppGrid items={visibleApps} onOpen={setSelectedId} />
      )}
      {selectedId && (
        <AppDetail
          appId={selectedId}
          onClose={closeDetail}
          onCatalogChanged={() => void refresh()}
          onPrimaryAction={handlePrimaryAction}
          onUpgrade={handleUpgrade}
        />
      )}
      {planApp && plan && (
        <InstallPlanDialog
          app={planApp}
          upgrade={plan.upgrade}
          onClose={closePlan}
          onInstalled={() => void refresh()}
          install={install}
        />
      )}
    </div>
  )
}

export default App
