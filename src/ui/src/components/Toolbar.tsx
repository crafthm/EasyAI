import { useState } from 'react'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { GearIcon, PlusIcon } from './icons'

export function Toolbar() {
  const [addOpen, setAddOpen] = useState(false)
  const [url, setUrl] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function handleAdd() {
    const value = url.trim()
    if (!value) {
      setError('Enter a GitHub repository URL.')
      return
    }
    if (!isTauri()) {
      setError(
        'Adding a repository requires the desktop app. From the repository root, run "pnpm dlx @tauri-apps/cli@^2 dev" instead of opening the browser preview.',
      )
      return
    }

    setBusy(true)
    setError(null)
    try {
      await invoke('add_repo', { url: value })
      setAddOpen(false)
      setUrl('')
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <header className="toolbar">
      <h1 className="brand">StartAI</h1>
      <div className="toolbar-actions">
        <div className="popover-anchor">
          <button
            type="button"
            className="icon-button"
            title="Add GitHub repo"
            aria-label="Add GitHub repo"
            aria-haspopup="dialog"
            aria-expanded={addOpen}
            onClick={() => {
              setAddOpen((open) => !open)
              setError(null)
            }}
          >
            <PlusIcon />
          </button>
          {addOpen && (
            <div className="popover" role="dialog" aria-label="Add GitHub repo">
              <div className="popover-row">
                <label className="popover-label" htmlFor="github-url">
                  GitHub URL:
                </label>
                <input
                  id="github-url"
                  className="popover-input"
                  type="text"
                  placeholder="https://github.com/user/repo"
                  value={url}
                  disabled={busy}
                  onChange={(event) => {
                    setUrl(event.target.value)
                    setError(null)
                  }}
                />
              </div>
              {error && (
                <p className="popover-error" role="alert">
                  {error}
                </p>
              )}
              <div className="popover-actions">
                <button
                  type="button"
                  className="popover-button"
                  disabled={busy}
                  onClick={() => setAddOpen(false)}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="popover-button popover-button--primary"
                  disabled={busy}
                  onClick={handleAdd}
                >
                  {busy ? 'Adding…' : 'OK'}
                </button>
              </div>
            </div>
          )}
        </div>
        <button type="button" className="icon-button" title="Settings" aria-label="Settings">
          <GearIcon />
        </button>
      </div>
    </header>
  )
}