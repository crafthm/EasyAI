import { useEffect, useState } from 'react'
import {
  planInstall,
  type AppSummary,
  type InstallPlan,
} from '../api/apps'

type Props = {
  app: AppSummary
  onClose: () => void
  onInstalled: () => void
  install: (
    appId: string,
    methodId: string | null,
    allowAdvanced: boolean,
    allowElevation: boolean,
  ) => Promise<void>
}

export function InstallPlanDialog({
  app,
  onClose,
  onInstalled,
  install,
}: Props) {
  const [plan, setPlan] = useState<InstallPlan | null>(null)
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [allowAdvanced, setAllowAdvanced] = useState(false)
  const [allowElevation, setAllowElevation] = useState(false)

  useEffect(() => {
    let disposed = false
    planInstall(app.id, null, allowAdvanced, allowElevation)
      .then((next) => {
        if (!disposed) setPlan(next)
      })
      .catch((err) => {
        if (!disposed) {
          setPlan(null)
          setError(String(err))
        }
      })
      .finally(() => {
        if (!disposed) setLoading(false)
      })
    return () => {
      disposed = true
    }
  }, [app.id, allowAdvanced, allowElevation])

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy) onClose()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [busy, onClose])

  async function handleInstall() {
    if (!plan) return
    setBusy(true)
    setError(null)
    try {
      await install(app.id, plan.methodId, allowAdvanced, allowElevation)
      onInstalled()
      onClose()
    } catch (err) {
      setError(String(err))
      setBusy(false)
    }
  }

  const adopt = plan?.action === 'adopt'

  return (
    <div
      className="import-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose()
      }}
    >
      <section
        className="import-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={`Install ${app.name}`}
      >
        <header className="import-header">
          <h2 className="import-title">Install {app.name}</h2>
          <button
            type="button"
            className="icon-button"
            aria-label="Close install plan"
            disabled={busy}
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        {loading && <p className="detail-muted">Resolving installation method…</p>}

        {error && (
          <p className="popover-error" role="alert">
            {error}
          </p>
        )}

        {plan && (
          <>
            <dl className="preview-grid">
              <dt>Method</dt>
              <dd>
                {plan.methodId} · {plan.methodKind}
              </dd>
              <dt>Tier</dt>
              <dd>{plan.tier}</dd>
              <dt>Score</dt>
              <dd>{plan.score}</dd>
              <dt>Estimated size</dt>
              <dd>
                {plan.estimatedSize
                  ? `${Math.round(plan.estimatedSize / 1_048_576)} MB`
                  : 'Unknown'}
              </dd>
              <dt>Elevation</dt>
              <dd>{plan.needsElevation ? 'Required' : 'Not required'}</dd>
              <dt>Action</dt>
              <dd>{plan.action}</dd>
            </dl>

            <div className="plan-section">
              <h3 className="detail-section-title">Why this method</h3>
              <ul className="plan-list">
                {plan.rationale.map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
            </div>

            {plan.existing.length > 0 && (
              <div className="plan-section">
                <h3 className="detail-section-title">Existing installations</h3>
                <ul className="plan-list">
                  {plan.existing.map((item) => (
                    <li key={`${item.kind}-${item.location}`}>
                      {item.kind} · {item.version ?? 'unknown version'} ·{' '}
                      {item.location}
                    </li>
                  ))}
                </ul>
              </div>
            )}

            {plan.rejected.length > 0 && (
              <div className="plan-section">
                <h3 className="detail-section-title">Not selected</h3>
                <ul className="plan-list plan-rejected">
                  {plan.rejected.map((item) => (
                    <li key={item.methodId}>
                      <strong>{item.methodId}</strong>: {item.reason}
                    </li>
                  ))}
                </ul>
              </div>
            )}

            <div className="plan-toggles">
              <label>
                <input
                  type="checkbox"
                  checked={allowAdvanced}
                  disabled={busy}
                  onChange={(event) => {
                    setAllowAdvanced(event.target.checked)
                    setLoading(true)
                    setError(null)
                  }}
                />
                Allow advanced methods (source/VCS/script)
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={allowElevation}
                  disabled={busy}
                  onChange={(event) => {
                    setAllowElevation(event.target.checked)
                    setLoading(true)
                    setError(null)
                  }}
                />
                Allow elevation
              </label>
            </div>

            {adopt && (
              <p className="preview-warning">
                An existing installation was found. Uninstall it or manage it
                before installing another copy.
              </p>
            )}
          </>
        )}

        <div className="import-actions">
          <button
            type="button"
            className="popover-button"
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="button"
            className="popover-button popover-button--primary"
            disabled={!plan || loading || busy || adopt}
            onClick={() => void handleInstall()}
          >
            {busy ? 'Installing…' : 'Install'}
          </button>
        </div>
      </section>
    </div>
  )
}