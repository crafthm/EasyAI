import { useEffect, useState } from 'react'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  getAppDetail,
  getJobLog,
  listJobs,
  onAppState,
  onJobFinished,
  onJobLog,
  openUrl,
  removeRecipe,
  uninstallApp,
  type AppDetail as AppDetailData,
  type JobRecord,
} from '../api/apps'
import { lifecycleLabel, primaryAction } from '../lifecycle'

type Props = {
  appId: string
  onClose: () => void
  onCatalogChanged: () => void
  onPrimaryAction: (appId: string) => void
}

export function AppDetail({
  appId,
  onClose,
  onCatalogChanged,
  onPrimaryAction,
}: Props) {
  const [detail, setDetail] = useState<AppDetailData | null>(null)
  const [jobs, setJobs] = useState<JobRecord[]>([])
  const [log, setLog] = useState<string | null>(null)
  const [liveLog, setLiveLog] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [copiedKey, setCopiedKey] = useState<string | null>(null)

  useEffect(() => {
    let disposed = false
    let unlistenLog: UnlistenFn | undefined
    let unlistenFinished: UnlistenFn | undefined
    let unlistenState: UnlistenFn | undefined

    getAppDetail(appId)
      .then((next) => {
        if (!disposed) setDetail(next)
      })
      .catch((err) => {
        if (!disposed) setError(String(err))
      })
    listJobs(appId)
      .then((next) => {
        if (!disposed) setJobs(next)
      })
      .catch((err) => {
        if (!disposed) setError(String(err))
      })

    onJobLog((event) => {
      if (event.appId === appId) {
        setLiveLog((current) => `${current}${event.line}\n`)
      }
    })
      .then((fn) => {
        if (disposed) fn()
        else unlistenLog = fn
      })
      .catch(() => {})

    onJobFinished((event) => {
      if (event.appId !== appId) return
      listJobs(appId)
        .then((next) => {
          if (!disposed) setJobs(next)
        })
        .catch(() => {})
      getAppDetail(appId)
        .then((next) => {
          if (!disposed) setDetail(next)
        })
        .catch(() => {})
    })
      .then((fn) => {
        if (disposed) fn()
        else unlistenFinished = fn
      })
      .catch(() => {})

    onAppState((event) => {
      if (event.appId !== appId) return
      setDetail((current) =>
        current
          ? {
              ...current,
              state: event.state,
              version:
                event.version ??
                (event.state === 'not_installed' ? null : current.version),
              managed: event.managed,
            }
          : current,
      )
    })
      .then((fn) => {
        if (disposed) fn()
        else unlistenState = fn
      })
      .catch(() => {})

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handleKeyDown)

    return () => {
      disposed = true
      unlistenLog?.()
      unlistenFinished?.()
      unlistenState?.()
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [appId, onClose])

  async function viewLog(job: JobRecord) {
    setBusy(true)
    setError(null)
    try {
      setLog(await getJobLog(job.id))
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  async function copyText(text: string, key: string) {
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      const area = document.createElement('textarea')
      area.value = text
      document.body.appendChild(area)
      area.select()
      document.execCommand('copy')
      area.remove()
    }
    setCopiedKey(key)
    window.setTimeout(() => {
      setCopiedKey((current) => (current === key ? null : current))
    }, 1500)
  }

  async function handleOpen(url: string) {
    try {
      await openUrl(appId, url)
    } catch (err) {
      setError(String(err))
    }
  }

  async function handleRemoveRecipe() {
    if (!detail) return
    const confirmed = window.confirm(
      `Remove "${detail.name}" from the catalog? Its imported recipe will be deleted.`,
    )
    if (!confirmed) return

    setBusy(true)
    setError(null)
    try {
      await removeRecipe(appId)
      onCatalogChanged()
      onClose()
    } catch (err) {
      setError(String(err))
      setBusy(false)
    }
  }

  async function handleUninstall() {
    if (!detail) return
    const confirmed = window.confirm(
      `Uninstall ${detail.name}? Installed files will be removed; models and other user data will be kept.`,
    )
    if (!confirmed) return

    setBusy(true)
    setError(null)
    try {
      await uninstallApp(appId)
      onClose()
    } catch (err) {
      setError(String(err))
      setBusy(false)
    }
  }

  const visibleLog = log ?? liveLog
  const usage = detail?.usage ?? null
  const action = detail ? primaryAction(detail) : null

  return (
    <div
      className="detail-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <section
        className="detail-panel"
        role="dialog"
        aria-modal="true"
        aria-label={detail ? `${detail.name} details` : 'Application details'}
      >
        <header className="detail-header">
          <h2 className="detail-title">{detail?.name ?? 'Loading…'}</h2>
          <button
            type="button"
            className="icon-button"
            aria-label="Close details"
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        {error && (
          <p className="popover-error" role="alert">
            {error}
          </p>
        )}

        {detail && (
          <>
            <p className="detail-description">{detail.description}</p>
            {detail.draft && (
              <p className="preview-warning">
                No install method yet. Imported from a repository without an
                EasyAI recipe.
              </p>
            )}
            <div className="detail-row">
              <span>State</span>
              <span>{lifecycleLabel(detail)}</span>
            </div>
            <div className="detail-row">
              <span>Version</span>
              <span>{detail.version ?? 'Not installed'}</span>
            </div>
            <div className="detail-row">
              <span>Install method</span>
              <span>
                {detail.methodId ?? detail.method ?? '—'}
                {detail.methodKind ? ` (${detail.methodKind})` : ''}
              </span>
            </div>
            <div className="detail-row">
              <span>Install path</span>
              <span className="detail-path">{detail.installDir ?? '—'}</span>
            </div>
            <div className="detail-row">
              <span>Health</span>
              <span>
                {detail.healthOk ? 'Responding' : 'Not responding'}
              </span>
            </div>
            {detail.sourceUrl && (
              <div className="detail-row">
                <span>Recipe source</span>
                <span className="detail-path">{detail.sourceUrl}</span>
              </div>
            )}
            {detail.bomPath && (
              <div className="detail-row">
                <span>Bill of materials</span>
                <span className="detail-path">{detail.bomPath}</span>
              </div>
            )}

            <div className="detail-actions">
              {action && (
                <button
                  type="button"
                  className="popover-button popover-button--primary"
                  disabled={busy || action.disabled}
                  onClick={() => onPrimaryAction(appId)}
                >
                  {action.label}
                </button>
              )}
              <button
                type="button"
                className="popover-button"
                disabled
                title="Configuration is not available in this version."
              >
                Configure
              </button>
              {detail.sourceUrl && detail.state === 'not_installed' && (
                <button
                  type="button"
                  className="popover-button"
                  disabled={busy}
                  onClick={() => void handleRemoveRecipe()}
                >
                  Remove from catalog
                </button>
              )}
              {detail.state !== 'not_installed' && (
                <button
                  type="button"
                  className="popover-button detail-danger"
                  disabled={busy}
                  onClick={() => void handleUninstall()}
                >
                  Uninstall
                </button>
              )}
            </div>
          </>
        )}

        {detail && detail.state !== 'not_installed' && usage && (
          <div className="usage-section">
            <h3 className="detail-section-title">Use</h3>
            {usage.ui_url && (
              <div className="usage-row">
                <span className="usage-label">Web UI</span>
                <code className="usage-value">{usage.ui_url}</code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  disabled={detail.state !== 'running'}
                  onClick={() => void handleOpen(usage.ui_url!)}
                >
                  Open
                </button>
              </div>
            )}
            {usage.api?.base_url && (
              <div className="usage-row">
                <span className="usage-label">API base</span>
                <code className="usage-value">{usage.api.base_url}</code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  onClick={() => void copyText(usage.api!.base_url!, 'api')}
                >
                  {copiedKey === 'api' ? 'Copied' : 'Copy'}
                </button>
              </div>
            )}
            {usage.api?.openai_base_url && (
              <div className="usage-row">
                <span className="usage-label">OpenAI-compatible</span>
                <code className="usage-value">{usage.api.openai_base_url}</code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  onClick={() => void copyText(usage.api!.openai_base_url!, 'openai')}
                >
                  {copiedKey === 'openai' ? 'Copied' : 'Copy'}
                </button>
              </div>
            )}
            {(usage.api?.docs_url || usage.docs_url) && (
              <div className="usage-row">
                <span className="usage-label">Documentation</span>
                <code className="usage-value">
                  {usage.api?.docs_url ?? usage.docs_url}
                </code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  onClick={() =>
                    void handleOpen((usage.api?.docs_url ?? usage.docs_url)!)
                  }
                >
                  Open
                </button>
              </div>
            )}
            {usage.cli.map((item) => (
              <div className="usage-row" key={item.command}>
                <span className="usage-label">{item.label}</span>
                <code className="usage-value">{item.command}</code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  onClick={() => void copyText(item.command, item.command)}
                >
                  {copiedKey === item.command ? 'Copied' : 'Copy'}
                </button>
              </div>
            ))}
            {usage.notes && <p className="usage-note">{usage.notes}</p>}
          </div>
        )}

        {detail?.provenanceJson && (
          <div className="plan-section">
            <h3 className="detail-section-title">Provenance</h3>
            <pre className="job-log">{detail.provenanceJson}</pre>
          </div>
        )}

        <h3 className="detail-section-title">Recent jobs</h3>
        {jobs.length === 0 ? (
          <p className="detail-muted">No jobs yet.</p>
        ) : (
          <ul className="job-list">
            {jobs.map((job) => (
              <li key={job.id} className="job-item">
                <div className="job-summary">
                  <span className={`job-state job-state--${job.state}`}>
                    {job.state}
                  </span>
                  <span className="job-kind">{job.kind}</span>
                  <span className="job-meta">
                    {job.origin} · {job.createdAt}
                  </span>
                  {job.error && <p className="job-error">{job.error}</p>}
                </div>
                <button
                  type="button"
                  className="popover-button"
                  disabled={busy}
                  onClick={() => void viewLog(job)}
                >
                  Log
                </button>
              </li>
            ))}
          </ul>
        )}

        {(log !== null || liveLog !== '') && (
          <>
            <h3 className="detail-section-title">Log</h3>
            <pre className="job-log">{visibleLog}</pre>
          </>
        )}
      </section>
    </div>
  )
}