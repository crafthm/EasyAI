import { useEffect, useRef, useState } from 'react'
import { CloseIcon, GearIcon, PlusIcon, SearchIcon } from './icons'
import { ImportDialog } from './ImportDialog'
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
  const [searchOpen, setSearchOpen] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (searchOpen) searchRef.current?.focus()
  }, [searchOpen])

  function closeSearch() {
    setSearchOpen(false)
    onQueryChange('')
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
    </>
  )
}
