import { useState } from 'react'
import { GearIcon, PlusIcon } from './icons'

export function Toolbar() {
  const [addOpen, setAddOpen] = useState(false)

  return (
    <header className="toolbar">
      <h1 className="brand">StartAI</h1>
      <div className="toolbar-actions">
        <div className="popover-anchor">
          <button
            type="button"
            className="icon-button"
            title="添加应用"
            aria-label="添加应用"
            aria-haspopup="dialog"
            aria-expanded={addOpen}
            onClick={() => setAddOpen((open) => !open)}
          >
            <PlusIcon />
          </button>
          {addOpen && (
            <div className="popover" role="dialog" aria-label="添加应用">
              <div className="popover-row">
                <label className="popover-label" htmlFor="github-url">
                  Github地址：
                </label>
                <input
                  id="github-url"
                  className="popover-input"
                  type="text"
                  placeholder="https://github.com/user/repo"
                />
              </div>
              <div className="popover-actions">
                <button
                  type="button"
                  className="popover-button"
                  onClick={() => setAddOpen(false)}
                >
                  取消
                </button>
                <button
                  type="button"
                  className="popover-button popover-button--primary"
                >
                  确定
                </button>
              </div>
            </div>
          )}
        </div>
        <button type="button" className="icon-button" title="设置" aria-label="设置">
          <GearIcon />
        </button>
      </div>
    </header>
  )
}