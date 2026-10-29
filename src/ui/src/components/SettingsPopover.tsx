import { useEffect, useState } from 'react'
import {
  getMethodSettings,
  saveMethodSettings,
  type MethodSettings,
} from '../api/apps'

type Props = {
  onClose: () => void
}

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
  const [settings, setSettings] = useState<MethodSettings | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState<string | null>(null)

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
        if (!disposed) setError(String(err))
      })
      .finally(() => {
        if (!disposed) setLoading(false)
      })
    return () => {
      disposed = true
    }
  }, [])

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
    setError(null)
    setSaved(false)
    try {
      await saveMethodSettings(settings)
      setSaved(true)
    } catch (err) {
      setError(String(err))
    } finally {
      setSaving(false)
    }
  }

  return (
    <div className="popover settings-popover" role="dialog" aria-label="Settings">
      <h2 className="popover-title">Settings</h2>
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

      {error && (
        <p className="popover-error" role="alert">
          {error}
        </p>
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
        <button type="button" className="popover-button" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  )
}