import { useEffect, useRef, useState } from 'react'
import type { UnlistenFn } from '@tauri-apps/api/event'
import {
  discoverUsage,
  getAppDetail,
  getJobLog,
  getUpdateStatus,
  getUsageReport,
  listJobs,
  onAppState,
  onJobFinished,
  onJobLog,
  onUpdateAvailable,
  openLocalUrl,
  openUrl,
  reinferUsage,
  removeApp,
  reportUiError,
  runUsageCheck,
  saveStartCommand,
  uninstallApp,
  type AppDetail as AppDetailData,
  type DetectedEntry,
  type JobRecord,
  type UpdateStatus,
  type UsageReport,
} from '../api/apps'
import { lifecycleLabel, primaryAction } from '../lifecycle'

const USAGE_STATE_LABEL: Record<string, string> = {
  verified: 'Verified',
  stale: 'Stale',
  failed: 'Failed',
  unverified: 'Unverified',
}

function UsageBadge({ state }: { state?: string }) {
  if (!state) return null
  return (
    <span className={`usage-badge usage-badge--${state}`}>
      {USAGE_STATE_LABEL[state] ?? state}
    </span>
  )
}

const MAX_LIVE_LOG_LINES = 2000

// Keeps the in-memory live transcript bounded during long installs.
function appendLiveLine(current: string, line: string): string {
  const next = `${current}${line}\n`
  const lines = next.split('\n')
  if (lines.length <= MAX_LIVE_LOG_LINES) return next
  return lines.slice(-MAX_LIVE_LOG_LINES).join('\n')
}

function slug(value: string): string {
  const slug = value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return slug || 'item'
}

function cliUsageId(item: { id: string | null; label: string }): string {
  return item.id ?? `cli-${slug(item.label)}`
}

type Props = {
  appId: string
  onClose: () => void
  onCatalogChanged: () => void
  onPrimaryAction: (appId: string) => void
  onUpgrade: (appId: string) => void
}

export function AppDetail({
  appId,
  onClose,
  onCatalogChanged,
  onPrimaryAction,
  onUpgrade,
}: Props) {
  const [detail, setDetail] = useState<AppDetailData | null>(null)
  const [jobs, setJobs] = useState<JobRecord[]>([])
  const [log, setLog] = useState<string | null>(null)
  const [liveLog, setLiveLog] = useState('')
  const [liveJobId, setLiveJobId] = useState<number | null>(null)
  const [selectedJobId, setSelectedJobId] = useState<number | null>(null)
  const liveJobIdRef = useRef<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const [usageReport, setUsageReport] = useState<UsageReport | null>(null)
  const [detected, setDetected] = useState<DetectedEntry[]>([])
  const [verifying, setVerifying] = useState(false)
  const [reanalyzing, setReanalyzing] = useState(false)
  const [update, setUpdate] = useState<UpdateStatus | null>(null)
  const [startCommandInput, setStartCommandInput] = useState('')

  useEffect(() => {
    let disposed = false
    let unlistenLog: UnlistenFn | undefined
    let unlistenFinished: UnlistenFn | undefined
    let unlistenState: UnlistenFn | undefined
    let unlistenUpdate: UnlistenFn | undefined

    getAppDetail(appId)
      .then((next) => {
        if (disposed) return
        setDetail(next)
        setStartCommandInput(next.startCommand?.join(' ') ?? '')
        if (next.state !== 'not_installed') {
          getUpdateStatus(appId)
            .then((status) => {
              if (!disposed) setUpdate(status)
            })
            .catch((err) => reportUiError('load update status', err))
          getUsageReport(appId)
            .then((report) => {
              if (!disposed) setUsageReport(report)
            })
            .catch((err) => reportUiError('load usage report', err))
          discoverUsage(appId)
            .then((entries) => {
              if (!disposed) setDetected(entries)
            })
            .catch((err) => reportUiError('discover usage', err))
        }
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
      if (event.appId !== appId) return
      const sameJob = liveJobIdRef.current === event.jobId
      // A new job takes over the view so the running transcript stays visible.
      if (!sameJob && liveJobIdRef.current !== null) {
        setSelectedJobId(null)
        setLog(null)
      }
      liveJobIdRef.current = event.jobId
      setLiveJobId(event.jobId)
      setLiveLog((current) => appendLiveLine(sameJob ? current : '', event.line))
    })
      .then((fn) => {
        if (disposed) fn()
        else unlistenLog = fn
      })
      .catch((err) => reportUiError('subscribe to job log', err))

    onJobFinished((event) => {
      if (event.appId !== appId) return
      listJobs(appId)
        .then((next) => {
          if (!disposed) setJobs(next)
        })
        .catch((err) => reportUiError('refresh jobs', err))
      getAppDetail(appId)
        .then((next) => {
          if (!disposed) setDetail(next)
        })
        .catch((err) => reportUiError('refresh app detail', err))
    })
      .then((fn) => {
        if (disposed) fn()
        else unlistenFinished = fn
      })
      .catch((err) => reportUiError('subscribe to job completion', err))

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
      if (event.state === 'not_installed') {
        setUsageReport(null)
        setDetected([])
      } else if (event.state === 'running') {
        discoverUsage(appId)
          .then((entries) => {
            if (!disposed) setDetected(entries)
          })
          .catch((err) => reportUiError('discover usage after start', err))
      }
    })
      .then((fn) => {
        if (disposed) fn()
        else unlistenState = fn
      })
      .catch((err) => reportUiError('subscribe to app state', err))

    onUpdateAvailable((event) => {
      if (event.appId !== appId) return
      setUpdate((current) => ({
        appId: event.appId,
        installedVersion: event.installedVersion,
        latestVersion: event.latestVersion,
        available: true,
        source: current?.source ?? 'github-release',
        checkedAt: current?.checkedAt ?? '',
      }))
    })
      .then((fn) => {
        if (disposed) fn()
        else unlistenUpdate = fn
      })
      .catch((err) => reportUiError('subscribe to update events', err))

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', handleKeyDown)

    return () => {
      disposed = true
      unlistenLog?.()
      unlistenFinished?.()
      unlistenState?.()
      unlistenUpdate?.()
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [appId, onClose])

  async function viewLog(job: JobRecord) {
    setBusy(true)
    setError(null)
    try {
      setSelectedJobId(job.id)
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

  async function handleOpenLocal(url: string) {
    try {
      await openLocalUrl(url)
    } catch (err) {
      setError(String(err))
    }
  }

  async function handleVerifyUsage() {
    setVerifying(true)
    setError(null)
    try {
      setUsageReport(await runUsageCheck(appId))
      setDetected(await discoverUsage(appId))
    } catch (err) {
      setError(String(err))
    } finally {
      setVerifying(false)
    }
  }

  async function handleReinferUsage() {
    setReanalyzing(true)
    setError(null)
    try {
      await reinferUsage(appId)
      setDetail(await getAppDetail(appId))
      setUsageReport(await getUsageReport(appId))
    } catch (err) {
      setError(String(err))
    } finally {
      setReanalyzing(false)
    }
  }

  async function handleSaveStartCommand() {
    if (!detail) return
    setBusy(true)
    setError(null)
    try {
      const command = startCommandInput.trim()
        ? startCommandInput.trim().split(/\s+/)
        : []
      await saveStartCommand(appId, command)
      setDetail(await getAppDetail(appId))
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  async function handleRemoveApp() {
    if (!detail) return
    const confirmed = window.confirm(
      detail.sourceUrl
        ? `Remove "${detail.name}" from the catalog? Its imported recipe will be deleted.`
        : `Remove "${detail.name}" from the catalog?`,
    )
    if (!confirmed) return

    setBusy(true)
    setError(null)
    try {
      await removeApp(appId)
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

  const visibleLog =
    selectedJobId === null
      ? liveLog
      : selectedJobId === liveJobId
        ? liveLog || log || ''
        : log ?? ''
  const usage = detail?.usage ?? null
  const action = detail ? primaryAction(detail) : null
  const usageStates = new Map(
    usageReport?.items.map((item) => [item.id, item.state]) ?? [],
  )
  const installDir = detail?.installDir ?? ''
  const expandCommand = (command: string) =>
    installDir ? command.replace('{install_dir}', installDir) : command

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
                No install method yet. Imported from a repository.
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
            {update && (
              <div className="detail-row">
                <span>Latest version</span>
                <span>
                  {update.latestVersion}
                  <span
                    className={`update-badge${
                      update.available ? '' : ' update-badge--current'
                    }`}
                  >
                    {update.available ? 'Update available' : 'Up to date'}
                  </span>
                </span>
              </div>
            )}
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
                {detail.healthMonitored
                  ? detail.healthOk
                    ? 'Responding'
                    : 'Not responding'
                  : 'Not monitored'}
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

            {detail.state !== 'not_installed' &&
              detail.methodKind !== 'container' && (
              <div className="plan-section">
                <h3 className="detail-section-title">Start command</h3>
                <p className="detail-description">
                  Optional. Overrides the command the Start action runs. Use{' '}
                  {'{install_dir}'} for the install directory.
                </p>
                <div className="import-row">
                  <input
                    className="popover-input"
                    type="text"
                    placeholder="{install_dir}/app serve"
                    value={startCommandInput}
                    disabled={busy}
                    onChange={(event) =>
                      setStartCommandInput(event.target.value)
                    }
                    onKeyDown={(event) => {
                      if (event.key === 'Enter' && !busy) {
                        void handleSaveStartCommand()
                      }
                    }}
                  />
                  <button
                    type="button"
                    className="popover-button"
                    disabled={busy}
                    onClick={() => void handleSaveStartCommand()}
                  >
                    Save
                  </button>
                </div>
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
              {detail.state !== 'not_installed' && (
                <button
                  type="button"
                  className="popover-button popover-button--primary"
                  disabled={busy || !update?.available}
                  title={
                    update?.available
                      ? `Update to ${update.latestVersion}`
                      : 'No update detected yet.'
                  }
                  onClick={() => onUpgrade(appId)}
                >
                  Update
                </button>
              )}
              {detail.state === 'not_installed' && (
                <button
                  type="button"
                  className="popover-button"
                  disabled={busy}
                  onClick={() => void handleRemoveApp()}
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
            <div className="usage-heading">
              <h3 className="detail-section-title">
                Use{' '}
                {usage.inferred && (
                  <span className="usage-badge usage-badge--inferred">
                    Inferred
                  </span>
                )}
              </h3>
              <div className="usage-heading-actions">
                {usageReport?.checkedAt && (
                  <span className="usage-checked">
                    Checked{' '}
                    {new Date(Number(usageReport.checkedAt) * 1000).toLocaleString()}
                  </span>
                )}
                {detail.sourceUrl && (
                  <button
                    type="button"
                    className="popover-button usage-action"
                    disabled={reanalyzing}
                    onClick={() => void handleReinferUsage()}
                  >
                    {reanalyzing ? 'Analyzing…' : 'Re-analyze'}
                  </button>
                )}
                <button
                  type="button"
                  className="popover-button usage-action"
                  disabled={detail.state !== 'running' || verifying}
                  onClick={() => void handleVerifyUsage()}
                >
                  {verifying ? 'Checking…' : 'Verify'}
                </button>
              </div>
            </div>
            {usage.website && (
              <div className="usage-row">
                <span className="usage-label">
                  Website <UsageBadge state={usageStates.get('website')} />
                </span>
                <code className="usage-value">{usage.website}</code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  onClick={() => void handleOpen(usage.website!)}
                >
                  Open
                </button>
              </div>
            )}
            {usage.ui_url && (
              <div className="usage-row">
                <span className="usage-label">
                  Web UI <UsageBadge state={usageStates.get('ui')} />
                </span>
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
                <span className="usage-label">
                  API base <UsageBadge state={usageStates.get('api-base')} />
                </span>
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
                <span className="usage-label">
                  OpenAI-compatible{' '}
                  <UsageBadge state={usageStates.get('openai-base')} />
                </span>
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
                <span className="usage-label">
                  Documentation{' '}
                  <UsageBadge
                    state={usageStates.get(
                      usage.api?.docs_url ? 'api-docs' : 'docs',
                    )}
                  />
                </span>
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
            {usage.launch && (
              <div className="usage-row">
                <span className="usage-label">
                  Launch <UsageBadge state={usageStates.get('launch')} />
                </span>
                <code className="usage-value">
                  {expandCommand(usage.launch.command)}
                </code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  onClick={() =>
                    void copyText(expandCommand(usage.launch!.command), 'launch')
                  }
                >
                  {copiedKey === 'launch' ? 'Copied' : 'Copy'}
                </button>
              </div>
            )}
            {usage.cli.map((item) => (
              <div className="usage-row" key={item.command}>
                <span className="usage-label">
                  {item.label}{' '}
                  <UsageBadge state={usageStates.get(cliUsageId(item))} />
                </span>
                <code className="usage-value">
                  {expandCommand(item.command)}
                </code>
                <button
                  type="button"
                  className="popover-button usage-action"
                  onClick={() =>
                    void copyText(expandCommand(item.command), item.command)
                  }
                >
                  {copiedKey === item.command ? 'Copied' : 'Copy'}
                </button>
              </div>
            ))}
            {usage.notes && <p className="usage-note">{usage.notes}</p>}
          </div>
        )}

        {detail && detail.state === 'running' && detected.length > 0 && (
          <div className="usage-section">
            <h3 className="detail-section-title">Detected</h3>
            {detected.map((entry) => (
              <div className="usage-row" key={entry.url}>
                <span className="usage-label">
                  {entry.kind}{' '}
                  <span
                    className={`usage-confidence usage-confidence--${entry.confidence}`}
                  >
                    {entry.confidence}
                  </span>
                </span>
                <code className="usage-value">{entry.url}</code>
                <span className="usage-detected-actions">
                  <button
                    type="button"
                    className="popover-button usage-action"
                    onClick={() => void handleOpenLocal(entry.url)}
                  >
                    Open
                  </button>
                  <button
                    type="button"
                    className="popover-button usage-action"
                    onClick={() => void copyText(entry.url, entry.url)}
                  >
                    {copiedKey === entry.url ? 'Copied' : 'Copy'}
                  </button>
                </span>
              </div>
            ))}
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

        {(selectedJobId !== null ? log !== null : liveLog !== '') && (
          <>
            <h3 className="detail-section-title">
              Log{selectedJobId !== null ? ` #${selectedJobId}` : ' (live)'}
            </h3>
            <pre className="job-log">{visibleLog}</pre>
          </>
        )}
      </section>
    </div>
  )
}