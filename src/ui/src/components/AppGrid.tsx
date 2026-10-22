import { useRef } from 'react'
import type { AppItem } from '../data/apps'
import { useGridLayout } from '../hooks/useGridLayout'

type Props = {
  items: AppItem[]
}

export function AppGrid({ items }: Props) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const { cols, rows } = useGridLayout(viewportRef, items.length)
  const placeholderCount = Math.max(0, cols * rows - items.length)

  return (
    <main className="grid-viewport" ref={viewportRef}>
      <div
        className="app-grid"
        style={{ gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }}
      >
        {items.map((item) => (
          <button
            key={item.id}
            type="button"
            className="app-tile"
            style={{ background: item.color }}
            title={item.name}
            aria-label={item.name}
          >
            <span aria-hidden="true">{item.glyph}</span>
          </button>
        ))}
        {Array.from({ length: placeholderCount }, (_, index) => (
          <div
            key={`empty-${index}`}
            className="app-tile app-tile--empty"
            aria-hidden="true"
          />
        ))}
      </div>
    </main>
  )
}