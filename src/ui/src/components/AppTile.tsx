import { useState } from 'react'
import type { AppSummary } from '../api/apps'
import { lifecycleLabel } from '../lifecycle'
import {
  AlertIcon,
  DownloadIcon,
  ExternalIcon,
  PlayIcon,
  SpinnerIcon,
  StopIcon,
} from './icons'

type ActionIcon = 'install' | 'start' | 'stop' | 'external' | 'busy' | 'failed'

type PrimaryAction = {
  icon: ActionIcon
  label: string
  disabled: boolean
}

function primaryAction(app: AppSummary): PrimaryAction {
  switch (app.state) {
    case 'not_installed':
      return { icon: 'install', label: 'Install', disabled: false }
    case 'stopped':
      return { icon: 'start', label: 'Start', disabled: false }
    case 'running':
      return app.managed
        ? { icon: 'stop', label: 'Stop', disabled: false }
        : { icon: 'external', label: 'Running externally', disabled: true }
    case 'installing':
      return { icon: 'busy', label: 'Installing', disabled: true }
    case 'starting':
      return { icon: 'busy', label: 'Starting', disabled: true }
    case 'stopping':
      return { icon: 'busy', label: 'Stopping', disabled: true }
    case 'uninstalling':
      return { icon: 'busy', label: 'Uninstalling', disabled: true }
    case 'failed':
      return { icon: 'failed', label: 'Failed', disabled: true }
  }
}

function ActionGlyph({ icon }: { icon: ActionIcon }) {
  switch (icon) {
    case 'install':
      return <DownloadIcon />
    case 'start':
      return <PlayIcon />
    case 'stop':
      return <StopIcon />
    case 'external':
      return <ExternalIcon />
    case 'busy':
      return <SpinnerIcon />
    case 'failed':
      return <AlertIcon />
  }
}

type Props = {
  app: AppSummary
  onOpen: (appId: string) => void
  onAction: (app: AppSummary) => void
}

export function AppTile({ app, onOpen, onAction }: Props) {
  const [logoFailed, setLogoFailed] = useState(false)
  const state = lifecycleLabel(app)
  const action = primaryAction(app)
  const logo = logoFailed ? null : app.iconUrl

  return (
    <div
      className="app-tile"
      style={{
        background: app.color || 'linear-gradient(135deg, #4b5563, #1f2937)',
      }}
    >
      {logo && (
        <img
          className="app-tile-logo"
          src={logo}
          alt=""
          aria-hidden="true"
          draggable={false}
          onError={() => setLogoFailed(true)}
        />
      )}
      <span className="app-tile-shade" aria-hidden="true" />
      <button
        type="button"
        className="app-tile-body"
        aria-label={`${app.name}, ${state}. Open details.`}
        onClick={() => onOpen(app.id)}
      >
        <span className="app-tile-name" title={app.name}>
          {app.name}
        </span>
        </button>
      <button
        type="button"
        className="app-tile-action"
        disabled={action.disabled}
        aria-label={`${action.label}: ${app.name}`}
        title={action.label}
        onClick={() => onAction(app)}
      >
        <ActionGlyph icon={action.icon} />
      </button>
    </div>
  )
}