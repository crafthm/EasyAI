import { useEffect, useRef, useState } from 'react'
import {
  getInstallRoot,
  getMethodSettings,
  saveInstallRoot,
  saveMethodSettings,
  type InstallRootInfo,
  type MethodSettings,
} from '../api/apps'

type Props = {
  onClose: () => void
}

type Tab = 'method' | 'storage'

const TABS: { id: Tab; label: string }[] = [
  { id: 'method', label: 'Install method' },
  { id: 'storage', label: 'Storage' },
]

const DEFAULT_ORDER = ['T0', 'T1', 'T2', 'T3', 'T4']

const TIER_INFO: Record<string, { title: string; detail: string }> = {
  T0: {
    title: 'Official desktop / portable / installer',
    detail: 'Lifecycle-capable official packaging',
  },
  T1: {
    title: 'Official release binary / container',
    detail: 'Release archives and official images',
  },
  T2: {
    title: 'Official package manager / language wheel',
    detail: 'brew, winget, pip/uv, npm, cargo, go',
  },
  T3: {
    title: 'Install script / community / distro package',
    detail: 'Install scripts, community taps, apt/dnf/pacman',
  },
  T4: {
    title: 'Source build / direct Git install',
    detail: 'Advanced; requires explicit opt-in',
  },
}

function defaults(): MethodSettings {
  return {
    order: [...DEFAULT_ORDER],
    enabled: Object.fromEntries(DEFAULT_ORDER.map((tier) => [tier, true])),
  }
}

export function SettingsPopover({ onClose }: Props) {
  const [tab, setTab] = useState<Tab>('method')
  const [error, setError] = useState<string | null>(null)
  const popoverRef = useRef<HTMLDivElement>(null)

  // Close on any press outside the popover. The gear button that opens this
  // lives outside the popover, so clicking it would otherwise close here and
  // immediately re-open from its own onClick. Listening in the capture phase
  // and ignoring presses inside `closest('[data-settings-anchor]')` keeps that
  // toggle working: the button remains the single owner of its own clicks.
  useEffect(() => {
    function handlePointerDown(event: MouseEvent | TouchEvent) {
      const target = event.target
      if (!(target instanceof Node)) return
      const anchor = popoverRef.current?.closest('[data-settings-anchor]')
      if (anchor?.contains(target)) return
      onClose()
    }

    document.addEventListener('mousedown', handlePointerDown, true)
    document.addEventListener('touchstart', handlePointerDown, true)
    return () => {
      document.removeEventListener('mousedown', handlePointerDown, true)
      document.removeEventListener('touchstart', handlePointerDown, true)
    }
  }, [onClose])

  // Escape closes it too; a popover with no visible close button would
  // otherwise be a keyboard trap.
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onClose()
    }
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [onClose])

  function handleTabKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
    event.preventDefault()
    const index = TABS.findIndex((item) => item.id === tab)
    const delta = event.key === 'ArrowRight' ? 1 : -1
    const next = TABS[(index + delta + TABS.length) % TABS.length]
    setTab(next.id)
    setError(null)
  }

  return (
    <div
      ref={popoverRef}
      className="popover settings-popover"
      role="dialog"
      aria-label="Settings"
    >
      <h2 className="popover-title">Settings</h2>

      <div
        className="settings-tabs"
        role="tablist"
        aria-label="Settings sections"
        onKeyDown={handleTabKeyDown}
      >
        {TABS.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            id={`settings-tab-${item.id}`}
            aria-selected={tab === item.id}
            aria-controls={`settings-panel-${item.id}`}
            tabIndex={tab === item.id ? 0 : -1}
            className={
              tab === item.id ? 'settings-tab settings-tab--active' : 'settings-tab'
            }
            onClick={() => {
              setTab(item.id)
              setError(null)
            }}
          >
            {item.label}
          </button>
        ))}
      </div>

      {error && (
        <p className="popover-error" role="alert">
          {error}
        </p>
      )}

      <div
        role="tabpanel"
        id={`settings-panel-${tab}`}
        aria-labelledby={`settings-tab-${tab}`}
      >
        {tab === 'method' ? (
          <MethodTab onError={setError} />
        ) : (
          <StorageTab onError={setError} />
        )}
      </div>
    </div>
  )
}

function MethodTab({ onError }: { onError: (message: string | null) => void }) {
  const [settings, setSettings] = useState<MethodSettings | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    let disposed = false
    getMethodSettings()
      .then((next) => {
        if (!disposed) {
          setSettings({
            order: next.order.length ? next.order : [...DEFAULT_ORDER],
            enabled: { ...defaults().enabled, ...next.enabled },
          })
        }
      })
      .catch((err) => {
        if (!disposed) onError(String(err))
      })
      .finally(() => {
        if (!disposed) setLoading(false)
      })
    return () => {
      disposed = true
    }
  }, [onError])

  function move(index: number, delta: number) {
    if (!settings) return
    const target = index + delta
    if (target < 0 || target >= settings.order.length) return
    const order = [...settings.order]
    ;[order[index], order[target]] = [order[target], order[index]]
    setSettings({ ...settings, order })
    setSaved(false)
  }

  function toggle(tier: string) {
    if (!settings) return
    setSettings({
      ...settings,
      enabled: {
        ...settings.enabled,
        [tier]: !(settings.enabled[tier] ?? true),
      },
    })
    setSaved(false)
  }

  async function save() {
    if (!settings) return
    setSaving(true)
    onError(null)
    setSaved(false)
    try {
      await saveMethodSettings(settings)
      setSaved(true)
    } catch (err) {
      onError(String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <>
      <p className="settings-hint">
        Installation method priority. Higher tiers are preferred; disabled tiers
        are never selected.
      </p>

      {loading ? (
        <p className="detail-muted">Loading…</p>
      ) : (
        settings && (
          <ol className="tier-list">
            {settings.order.map((tier, index) => {
              const info = TIER_INFO[tier] ?? { title: tier, detail: '' }
              const enabled = settings.enabled[tier] ?? true
              return (
                <li key={tier} className="tier-row">
                  <span className="tier-order">{tier}</span>
                  <span className="tier-copy">
                    <span className="tier-title">{info.title}</span>
                    <span className="tier-detail">{info.detail}</span>
                  </span>
                  <span className="tier-actions">
                    <button
                      type="button"
                      aria-label={`Move ${tier} up`}
                      disabled={saving || index === 0}
                      onClick={() => move(index, -1)}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      aria-label={`Move ${tier} down`}
                      disabled={saving || index === settings.order.length - 1}
                      onClick={() => move(index, 1)}
                    >
                      ↓
                    </button>
                    <label className="tier-toggle">
                      <input
                        type="checkbox"
                        checked={enabled}
                        disabled={saving}
                        onChange={() => toggle(tier)}
                      />
                      Enabled
                    </label>
                  </span>
                </li>
              )
            })}
          </ol>
        )
      )}

      {saved && (
        <p className="settings-saved">
          Saved. New install plans use this order.
        </p>
      )}

      <div className="popover-actions">
        <button
          type="button"
          className="popover-button"
          disabled={saving || loading}
          onClick={() => {
            setSettings(defaults())
            setSaved(false)
          }}
        >
          Reset
        </button>
        <button
          type="button"
          className="popover-button popover-button--primary"
          disabled={saving || !settings}
          onClick={() => void save()}
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
      </div>
    </>
  )
}

function StorageTab({ onError }: { onError: (message: string | null) => void }) {
  const [info, setInfo] = useState<InstallRootInfo | null>(null)
  const [draft, setDraft] = useState('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    let disposed = false
    getInstallRoot()
      .then((next) => {
        if (!disposed) {
          setInfo(next)
          setDraft(next.configured)
        }
      })
      .catch((err) => {
        if (!disposed) onError(String(err))
      })
      .finally(() => {
        if (!disposed) setLoading(false)
      })
    return () => {
      disposed = true
    }
  }, [onError])

  async function save() {
    setSaving(true)
    onError(null)
    setSaved(false)
    try {
      const next = await saveInstallRoot(draft)
      setInfo(next)
      setDraft(next.configured)
      setSaved(true)
    } catch (err) {
      onError(String(err))
    } finally {
      setSaving(false)
    }
  }

  const dirty = info !== null && draft.trim() !== info.configured.trim()

  return (
    <>
      <p className="settings-hint">
        Where new applications are installed. Existing applications keep their
        current location, so changing this only affects future installs.
      </p>

      {loading ? (
        <p className="detail-muted">Loading…</p>
      ) : (
        info && (
          <>
            <div className="settings-field">
              <label className="settings-field-label" htmlFor="install-root">
                Install root
              </label>
              <input
                id="install-root"
                className="settings-input"
                type="text"
                spellCheck={false}
                autoComplete="off"
                value={draft}
                placeholder={info.defaultRoot}
                disabled={saving}
                onChange={(event) => {
                  setDraft(event.target.value)
                  setSaved(false)
                }}
              />
              <p className="settings-field-hint">
                Absolute path. Leave empty to use the default:{' '}
                <span className="detail-path">{info.defaultRoot}</span>
              </p>
            </div>

            <div className="detail-row">
              <span>In use</span>
              <span className="detail-path">{info.root}</span>
            </div>

            {info.restartRequired && (
              <p className="settings-warning">
                Restart EasyAI to start installing into the saved location.
                Until then, installs continue to use{' '}
                <span className="detail-path">{info.root}</span>.
              </p>
            )}
          </>
        )
      )}

      {saved && <p className="settings-saved">Saved.</p>}

      <div className="popover-actions">
        <button
          type="button"
          className="popover-button"
          disabled={saving || loading || !info}
          onClick={() => {
            setDraft('')
            setSaved(false)
          }}
        >
          Reset
        </button>
        <button
          type="button"
          className="popover-button popover-button--primary"
          disabled={saving || loading || !dirty}
          onClick={() => void save()}
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
      </div>
    </>
  )
}
