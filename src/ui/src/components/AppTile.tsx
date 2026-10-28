import type { AppSummary } from '../api/apps'
import { lifecycleLabel } from '../lifecycle'

type PrimaryAction = {
  label: string
  disabled: boolean
}

function primaryAction(app: AppSummary): PrimaryAction {
  switch (app.state) {
    case 'not_installed':
      return { label: 'Install', disabled: false }
    case 'stopped':
      return { label: 'Start', disabled: false }
    case 'running':
      return app.managed
        ? { label: 'Stop', disabled: false }
        : { label: 'External', disabled: true }
    case 'installing':
      return { label: 'Installing…', disabled: true }
    case 'starting':
      return { label: 'Starting…', disabled: true }
    case 'stopping':
      return { label: 'Stopping…', disabled: true }
    case 'uninstalling':
      return { label: 'Uninstalling…', disabled: true }
    case 'failed':
      return { label: 'Failed', disabled: true }
  }
}

type Props = {
  app: AppSummary
  onOpen: (appId: string) => void
  onAction: (app: AppSummary) => void
}

export function AppTile({ app, onOpen, onAction }: Props) {
  const state = lifecycleLabel(app)
  const action = primaryAction(app)

  return (
    <div
      className="app-tile"
      style={{
        background: app.color || 'linear-gradient(135deg, #4b5563, #1f2937)',
      }}
    >
      <button
        type="button"
        className="app-tile-body"
        aria-label={`${app.name}, ${state}. Open details.`}
        onClick={() => onOpen(app.id)}
      >
        <span className="app-tile-glyph" aria-hidden="true">
          {app.glyph || app.name.slice(0, 1)}
        </span>
        <span className="app-tile-name">{app.name}</span>
        <span className="app-tile-state">{state}</span>
      </button>
      <button
        type="button"
        className="app-tile-action"
        disabled={action.disabled}
        aria-label={`${action.label}: ${app.name}`}
        onClick={() => onAction(app)}
      >
        {action.label}
      </button>
    </div>
  )
}