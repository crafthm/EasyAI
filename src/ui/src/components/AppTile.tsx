import { useState } from 'react'
import type { AppSummary } from '../api/apps'
import { lifecycleLabel } from '../lifecycle'

type Props = {
  app: AppSummary
  onOpen: (appId: string) => void
}

export function AppTile({ app, onOpen }: Props) {
  const [logoFailed, setLogoFailed] = useState(false)
  const state = lifecycleLabel(app)
  const logo = logoFailed ? null : app.iconUrl
  const running = app.state === 'running'

  return (
    <div className="app-tile">
      <button
        type="button"
        className="app-tile-button"
        aria-label={`${app.name}, ${state}. Open details.`}
        onClick={() => onOpen(app.id)}
      >
        <span
          className="app-tile-art"
          style={{
            background: logo
              ? 'transparent'
              : app.color || 'linear-gradient(135deg, #4b5563, #1f2937)',
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
          {!logo && <span className="app-tile-shade" aria-hidden="true" />}
          <span
            className={`app-tile-status ${
              running ? 'app-tile-status--up' : 'app-tile-status--down'
            }`}
            aria-hidden="true"
          />
        </span>
        <span className="app-tile-name" title={app.name}>
          {app.name}
        </span>
      </button>
    </div>
  )
}