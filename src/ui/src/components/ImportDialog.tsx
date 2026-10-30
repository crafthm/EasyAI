import { useEffect, useState } from 'react'
import {
  importRecipe,
  previewRecipe,
  type RecipePreview,
} from '../api/apps'

type Props = {
  onClose: () => void
  onImported: () => void
}

export function ImportDialog({ onClose, onImported }: Props) {
  const [url, setUrl] = useState('')
  const [preview, setPreview] = useState<RecipePreview | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy) onClose()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [busy, onClose])

  async function handlePreview() {
    const value = url.trim()
    if (!value) {
      setError('Enter a recipe or GitHub repository URL.')
      return
    }
    setBusy(true)
    setError(null)
    setPreview(null)
    try {
      setPreview(await previewRecipe(value))
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  async function handleImport() {
    if (!preview) return
    setBusy(true)
    setError(null)
    try {
      await importRecipe(preview.sourceUrl, preview.sha256)
      onImported()
      onClose()
    } catch (err) {
      setError(String(err))
      setBusy(false)
    }
  }

  const methods = preview?.recipe.methods ?? []
  const draft = preview?.recipe.draft ?? false
  const [os, arch] = preview ? preview.platform.split('-') : ['', '']
  const currentMethod = methods.find(
    (method) => method.when.os.includes(os) && method.when.arch.includes(arch),
  )
  const currentAsset = currentMethod?.source?.asset[preview?.platform ?? ''] ?? null
  const currentService = currentMethod?.service ?? null
  const canImport = Boolean(
    preview && preview.supported && !preview.conflict && !busy,
  )

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
        aria-label="Import application"
      >
        <header className="import-header">
          <h2 className="import-title">Import application</h2>
          <button
            type="button"
            className="icon-button"
            aria-label="Close import dialog"
            disabled={busy}
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        <p className="detail-description">
          Paste a recipe URL, or a GitHub repository.
        </p>

        <div className="import-row">
          <input
            className="popover-input"
            type="text"
            placeholder="https://github.com/owner/repo"
            value={url}
            disabled={busy}
            onChange={(event) => {
              setUrl(event.target.value)
              setPreview(null)
              setError(null)
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !busy) void handlePreview()
            }}
          />
          <button
            type="button"
            className="popover-button popover-button--primary"
            disabled={busy || !url.trim()}
            onClick={() => void handlePreview()}
          >
            {busy && !preview ? 'Fetching…' : 'Fetch'}
          </button>
        </div>

        {error && (
          <p className="popover-error" role="alert">
            {error}
          </p>
        )}

        {preview && (
          <div className="preview-section">
            <h3 className="preview-name">{preview.recipe.name}</h3>
            <p className="preview-description">{preview.recipe.description}</p>

            <dl className="preview-grid">
              <dt>Source</dt>
              <dd className="preview-mono">{preview.sourceUrl}</dd>
              <dt>Checksum</dt>
              <dd className="preview-mono">{preview.sha256.slice(0, 16)}…</dd>
              <dt>Platforms</dt>
              <dd>{preview.recipe.platforms.join(', ') || '—'}</dd>
              <dt>Type</dt>
              <dd>
                {draft
                  ? 'Draft · no install method'
                  : `${currentMethod?.kind ?? methods[0]?.kind ?? 'release'} · ${methods.length} platform${methods.length === 1 ? '' : 's'}`}
              </dd>
              {!draft && (
                <>
                  <dt>Asset ({preview.platform})</dt>
                  <dd className="preview-mono">{currentAsset ?? '—'}</dd>
                  <dt>Start command</dt>
                  <dd className="preview-mono">
                    {currentService
                      ? currentService.command.join(' ')
                      : 'Not inferred'}
                  </dd>
                </>
              )}
            </dl>

            {preview.conflict && (
              <p className="preview-warning preview-blocked">
                An application with this id already exists.
              </p>
            )}
            {preview.warnings.map((warning) => (
              <p key={warning} className="preview-warning">
                {warning}
              </p>
            ))}
          </div>
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
            disabled={!canImport}
            onClick={() => void handleImport()}
          >
            {busy && preview ? 'Adding…' : 'Add to catalog'}
          </button>
        </div>
      </section>
    </div>
  )
}