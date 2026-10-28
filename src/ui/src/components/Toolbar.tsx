import { useState } from 'react'
import { GearIcon, PlusIcon } from './icons'
import { ImportDialog } from './ImportDialog'
import { SettingsPopover } from './SettingsPopover'

type Props = {
  onImported: () => void
}

export function Toolbar({ onImported }: Props) {
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [importOpen, setImportOpen] = useState(false)

  return (
    <>
      <header className="toolbar">
        <h1 className="brand">EasyAI</h1>
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
          <div className="popover-anchor">
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