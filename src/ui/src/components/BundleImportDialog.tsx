import { useEffect, useState } from 'react'
import { importBundle, installApp, type BundleEntry } from '../api/apps'

type Item = BundleEntry & {
  phase: 'pending' | 'installing' | 'done' | 'failed'
  error?: string
}

type Props = {
  fileName: string
  json: string
  entries: BundleEntry[]
  onClose: () => void
  onFinished: () => void
}

function statusText(item: Item): string {
  if (item.phase === 'installing') return 'Installing…'
  if (item.phase === 'done') return 'Installed'
  if (item.phase === 'failed') return `Failed: ${item.error ?? 'unknown error'}`
  if (item.status === 'installed') return 'Already installed'
  if (item.status === 'unsupported') return item.message ?? 'Unsupported'
  return 'Will install'
}

export function BundleImportDialog({
  fileName,
  json,
  entries,
  onClose,
  onFinished,
}: Props) {
  const [items, setItems] = useState<Item[]>(() =>
    entries.map((entry) => ({ ...entry, phase: 'pending' })),
  )
  const [busy, setBusy] = useState(false)
  const [finished, setFinished] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy) onClose()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [busy, onClose])

  const pending = items.filter(
    (item) => item.status === 'install' && item.phase === 'pending',
  ).length

  async function handleInstall() {
    if (finished) {
      onClose()
      return
    }
    setBusy(true)
    setError(null)
    let current = items
    let importedAll = false
    try {
      const imported = await importBundle(json)
      current = imported.map((entry) => ({ ...entry, phase: 'pending' }))
      setItems(current)

      for (let index = 0; index < current.length; index += 1) {
        const entry = current[index]
        if (entry.status !== 'install') continue

        current = current.map((item, itemIndex) =>
          itemIndex === index ? { ...item, phase: 'installing' } : item,
        )
        setItems(current)
        try {
          await installApp(entry.id, entry.methodId, true, false)
          current = current.map((item, itemIndex) =>
            itemIndex === index ? { ...item, phase: 'done' } : item,
          )
        } catch (err) {
          current = current.map((item, itemIndex) =>
            itemIndex === index
              ? { ...item, phase: 'failed', error: String(err) }
              : item,
          )
        }
        setItems(current)
      }
      importedAll = true
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
      if (importedAll) setFinished(true)
      onFinished()
    }
  }

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
        aria-label="Import easyai.json"
      >
        <header className="import-header">
          <h2 className="import-title">Import easyai.json</h2>
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
          {fileName} · {items.length} application{items.length === 1 ? '' : 's'}
        </p>

        {error && (
          <p className="popover-error" role="alert">
            {error}
          </p>
        )}

        <ul className="bundle-list">
          {items.map((item) => (
            <li key={item.id} className="bundle-item">
              <div className="bundle-item-main">
                <span className="bundle-item-name">{item.name}</span>
                <span className="bundle-item-meta">
                  {[item.version, item.methodId].filter(Boolean).join(' · ') ||
                    item.id}
                </span>
              </div>
              <span
                className={`bundle-item-status${
                  item.phase === 'failed' ? ' bundle-item-status--failed' : ''
                }`}
              >
                {statusText(item)}
              </span>
            </li>
          ))}
        </ul>

        <div className="import-actions">
          <button
            type="button"
            className="popover-button"
            disabled={busy}
            onClick={onClose}
          >
            {finished ? 'Close' : 'Cancel'}
          </button>
          {!finished && (
            <button
              type="button"
              className="popover-button popover-button--primary"
              disabled={busy || pending === 0}
              onClick={() => void handleInstall()}
            >
              {busy ? 'Installing…' : `Install ${pending || ''}`.trim()}
            </button>
          )}
        </div>
      </section>
    </div>
  )
}
