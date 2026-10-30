import { useLayoutEffect, useState, type RefObject } from 'react'

const GAP = 16
const MIN_CELL = 96
// 方形图标下方的名称与上下留白
const TILE_EXTRA = 38

export type GridLayout = {
  cols: number
  rows: number
}

export function useGridLayout(
  ref: RefObject<HTMLElement | null>,
  itemCount: number,
): GridLayout {
  const [layout, setLayout] = useState<GridLayout>({ cols: 1, rows: 1 })

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return

    const update = (width: number, height: number) => {
      if (width <= 0 || height <= 0) return

      const cols = Math.max(1, Math.floor((width + GAP) / (MIN_CELL + GAP)))
      const cell = (width - GAP * (cols - 1)) / cols
      const visibleRows = Math.max(
        1,
        Math.floor((height + GAP) / (cell + TILE_EXTRA + GAP)),
      )
      // 内容超过一屏时补行，避免格子被裁掉
      const rows = Math.max(visibleRows, Math.ceil(itemCount / cols))

      setLayout((prev) =>
        prev.cols === cols && prev.rows === rows ? prev : { cols, rows },
      )
    }

    // 首帧同步测量（contentRect 不含 padding，这里手动扣掉）
    const style = getComputedStyle(el)
    update(
      el.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
      el.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom),
    )

    const observer = new ResizeObserver(([entry]) => {
      update(entry.contentRect.width, entry.contentRect.height)
    })
    observer.observe(el)

    return () => observer.disconnect()
  }, [ref, itemCount])

  return layout
}