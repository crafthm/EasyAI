import { useEffect, useState } from 'react'
import type { AppSummary } from '../api/apps'
import { lifecycleLabel } from '../lifecycle'

type Props = {
  app: AppSummary
  onOpen: (appId: string) => void
}

const ICON_ANALYSIS_SIZE = 256
const ICON_BACKGROUND_THRESHOLD = 220

// Icons often ship with large transparent or white margins (for example the
// BeefTV app-icon.svg), so scaling the raw image leaves the mark small inside
// a fixed tile. Crop those margins and let CSS fit the remaining mark.
function trimIconMargins(image: HTMLImageElement): string | null {
  try {
    const size = ICON_ANALYSIS_SIZE
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const context = canvas.getContext('2d', { willReadFrequently: true })
    if (!context) return null

    const scale = Math.min(size / image.naturalWidth, size / image.naturalHeight)
    const width = image.naturalWidth * scale
    const height = image.naturalHeight * scale
    context.drawImage(image, (size - width) / 2, (size - height) / 2, width, height)

    const { data } = context.getImageData(0, 0, size, size)
    let minX = size
    let minY = size
    let maxX = -1
    let maxY = -1
    for (let y = 0; y < size; y += 1) {
      for (let x = 0; x < size; x += 1) {
        const index = (y * size + x) * 4
        if (data[index + 3] < 16) continue
        if (
          data[index] >= ICON_BACKGROUND_THRESHOLD &&
          data[index + 1] >= ICON_BACKGROUND_THRESHOLD &&
          data[index + 2] >= ICON_BACKGROUND_THRESHOLD
        ) {
          continue
        }
        if (x < minX) minX = x
        if (x > maxX) maxX = x
        if (y < minY) minY = y
        if (y > maxY) maxY = y
      }
    }
    if (maxX < 0) return null

    const cropWidth = maxX - minX + 1
    const cropHeight = maxY - minY + 1
    if (cropWidth >= size * 0.98 && cropHeight >= size * 0.98) return null

    const cropped = document.createElement('canvas')
    cropped.width = cropWidth
    cropped.height = cropHeight
    const croppedContext = cropped.getContext('2d')
    if (!croppedContext) return null
    croppedContext.drawImage(
      canvas,
      minX,
      minY,
      cropWidth,
      cropHeight,
      0,
      0,
      cropWidth,
      cropHeight,
    )
    return cropped.toDataURL('image/png')
  } catch {
    // Cross-origin images without CORS cannot be inspected; keep the original.
    return null
  }
}

export function AppTile({ app, onOpen }: Props) {
  const [logoFailed, setLogoFailed] = useState(false)
  const state = lifecycleLabel(app)
  const logo = logoFailed ? null : app.iconUrl
  const [processed, setProcessed] = useState<{
    source: string
    src: string
  } | null>(null)
  const logoSrc = logo && processed?.source === logo ? processed.src : logo
  const running = app.state === 'running'

  useEffect(() => {
    if (!logo) return

    let cancelled = false
    const image = new Image()
    image.crossOrigin = 'anonymous'
    image.onload = () => {
      if (cancelled) return
      const trimmed = trimIconMargins(image)
      if (trimmed) setProcessed({ source: logo, src: trimmed })
    }
    image.src = logo

    return () => {
      cancelled = true
    }
  }, [logo])

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
              ? '#fff'
              : app.color || 'linear-gradient(135deg, #4b5563, #1f2937)',
          }}
        >
          {logoSrc && (
            <img
              className="app-tile-logo"
              src={logoSrc}
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