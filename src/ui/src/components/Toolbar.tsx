import { useEffect, useRef, useState, type ChangeEvent } from 'react'
import { exportBundle, previewBundle, type BundleEntry } from '../api/apps'
import {
  CloseIcon,
  ExportIcon,
  GearIcon,
  ImportIcon,
  PlusIcon,
  RadarIcon,
  SearchIcon,
} from './icons'
import { BundleImportDialog } from './BundleImportDialog'
import { ImportDialog } from './ImportDialog'
import { NetworkScanDialog } from './NetworkScanDialog'
import { SettingsPopover } from './SettingsPopover'

export type InstallFilter =
  | 'all'
  | 'installed'
  | 'not_installed'
  | 'running'
  | 'update_available'

type Props = {
  onImported: () => void
  query: string
  onQueryChange: (value: string) => void
  filter: InstallFilter
  onFilterChange: (value: InstallFilter) => void
}

const FILTERS: { value: InstallFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'installed', label: 'Installed' },
  { value: 'not_installed', label: 'Not installed' },
  { value: 'running', label: 'Running' },
  { value: 'update_available', label: 'Update available' },
]

export function Toolbar({
  onImported,
  query,
  onQueryChange,
  filter,
  onFilterChange,
}: Props) {
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [importOpen, setImportOpen] = useState(false)
  const [scanOpen, setScanOpen] = useState(false)
  const [searchOpen, setSearchOpen] = useState(false)
  const [bundle, setBundle] = useState<{
    fileName: string
    json: string
    entries: BundleEntry[]
  } | null>(null)
  const [actionBusy, setActionBusy] = useState(false)
  const [notice, setNotice] = useState<{
    kind: 'info' | 'error'
    text: string
  } | null>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (searchOpen) searchRef.current?.focus()
  }, [searchOpen])

  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(null), 6000)
    return () => window.clearTimeout(timer)
  }, [notice])

  function closeSearch() {
    setSearchOpen(false)
    onQueryChange('')
  }

  async function handleImportFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return

    setActionBusy(true)
    setNotice(null)
    try {
      const json = await file.text()
      const entries = await previewBundle(json)
      setBundle({ fileName: file.name, json, entries })
      setImportOpen(false)
      setSettingsOpen(false)
    } catch (err) {
      setNotice({ kind: 'error', text: String(err) })
    } finally {
      setActionBusy(false)
    }
  }

  async function handleExport() {
    setActionBusy(true)
    setNotice(null)
    try {
      const path = await exportBundle()
      if (path) setNotice({ kind: 'info', text: `Exported to ${path}` })
    } catch (err) {
      setNotice({ kind: 'error', text: String(err) })
    } finally {
      setActionBusy(false)
    }
  }

  return (
    <>
      <header className="toolbar">
        <h1 className="brand">EasyAI</h1>
        <div className="toolbar-search">
          {searchOpen ? (
            <div className="search-field">
              <SearchIcon />
              <input
                ref={searchRef}
                type="search"
                className="toolbar-search-input"
                placeholder="Search applications"
                aria-label="Search applications"
                value={query}
                onChange={(event) => onQueryChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') closeSearch()
                }}
              />
              <button
                type="button"
                className="search-clear"
                title="Close search"
                aria-label="Close search"
                onClick={closeSearch}
              >
                <CloseIcon />
              </button>
            </div>
          ) : (
            <button
              type="button"
              className="icon-button"
              title="Search applications"
              aria-label="Search applications"
              onClick={() => setSearchOpen(true)}
            >
              <SearchIcon />
            </button>
          )}
        </div>
        <div className="toolbar-filters">
          <select
            className="filter-select"
            aria-label="Filter applications"
            value={filter}
            onChange={(event) =>
              onFilterChange(event.target.value as InstallFilter)
            }
          >
            {FILTERS.map((item) => (
              <option key={item.value} value={item.value}>
                {item.label}
              </option>
            ))}
          </select>
        </div>
        <div className="toolbar-actions">
          <button
            type="button"
            className="icon-button"
            title="Scan local network"
            aria-label="Scan local network"
            aria-haspopup="dialog"
            aria-expanded={scanOpen}
            onClick={() => {
              setScanOpen(true)
              setImportOpen(false)
              setSettingsOpen(false)
            }}
          >
            <RadarIcon />
          </button>
          <button
            type="button"
            className="icon-button"
            title="Import easyai.json"
            aria-label="Import easyai.json"
            disabled={actionBusy}
            onClick={() => {
              setImportOpen(false)
              setSettingsOpen(false)
              fileRef.current?.click()
            }}
          >
            <ImportIcon />
          </button>
          <button
            type="button"
            className="icon-button"
            title="Export easyai.json"
            aria-label="Export easyai.json"
            disabled={actionBusy}
            onClick={() => void handleExport()}
          >
            <ExportIcon />
          </button>
          <input
            ref={fileRef}
            type="file"
            accept=".json,application/json"
            hidden
            onChange={(event) => void handleImportFile(event)}
          />
          <button
            type="button"
            className="icon-button"
            title="Import application"
            aria-label="Import application"
            aria-haspopup="dialog"
            aria-expanded={importOpen}
            onClick={() => {
              setImportOpen(true)
              setSettingsOpen(false)
            }}
          >
            <PlusIcon />
          </button>
          {/* data-settings-anchor marks the button + popover as one unit, so
              the popover's outside-click handler lets the gear toggle. */}
          <div className="popover-anchor" data-settings-anchor>
            <button
              type="button"
              className="icon-button"
              title="Settings"
              aria-label="Settings"
              aria-haspopup="dialog"
              aria-expanded={settingsOpen}
              onClick={() => {
                setSettingsOpen((open) => !open)
                setImportOpen(false)
              }}
            >
              <GearIcon />
            </button>
            {settingsOpen && (
              <SettingsPopover onClose={() => setSettingsOpen(false)} />
            )}
          </div>
        </div>
      </header>
      {importOpen && (
        <ImportDialog
          onClose={() => setImportOpen(false)}
          onImported={onImported}
        />
      )}
      {scanOpen && <NetworkScanDialog onClose={() => setScanOpen(false)} />}
      {bundle && (
        <BundleImportDialog
          fileName={bundle.fileName}
          json={bundle.json}
          entries={bundle.entries}
          onClose={() => setBundle(null)}
          onFinished={onImported}
        />
      )}
      {notice && (
        <div
          className={`toolbar-toast${
            notice.kind === 'error' ? ' toolbar-toast--error' : ''
          }`}
          role={notice.kind === 'error' ? 'alert' : 'status'}
        >
          {notice.text}
        </div>
      )}
    </>
  )
}
