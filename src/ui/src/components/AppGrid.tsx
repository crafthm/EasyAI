import { useRef } from 'react'
import type { AppSummary } from '../api/apps'
import { useGridLayout } from '../hooks/useGridLayout'
import { AppTile } from './AppTile'

type Props = {
  items: AppSummary[]
  onOpen: (appId: string) => void
}

export function AppGrid({ items, onOpen }: Props) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const { cols, rows } = useGridLayout(viewportRef, items.length)
  const placeholderCount = Math.max(0, cols * rows - items.length)

  return (
    <main className="grid-viewport" ref={viewportRef}>
      {items.length === 0 ? (
        <p className="grid-message">No applications in the catalog.</p>
      ) : (
        <div
          className="app-grid"
          style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
        >
          {items.map((item) => (
            <AppTile key={item.id} app={item} onOpen={onOpen} />
          ))}
          {Array.from({ length: placeholderCount }, (_, index) => (
            <div
              key={`empty-${index}`}
              className="app-tile app-tile--empty"
              aria-hidden="true"
            />
          ))}
        </div>
      )}
    </main>
  )
}