import { useEffect, useState } from 'react'
import { scanLan, type LanScanResult } from '../api/apps'

type Props = {
  onClose: () => void
}

export function NetworkScanDialog({ onClose }: Props) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [keyPath, setKeyPath] = useState('')
  const [port, setPort] = useState('22')
  const [result, setResult] = useState<LanScanResult | null>(null)
  const [busy, setBusy] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy) onClose()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [busy, onClose])

  // Show something immediately: a credential-free sweep reports hosts and
  // banner-derived OS names while the user fills in the SSH fields.
  useEffect(() => {
    let cancelled = false
    scanLan(null)
      .then((scan) => {
        if (!cancelled) setResult(scan)
      })
      .catch((err) => {
        if (!cancelled) setError(String(err))
      })
      .finally(() => {
        if (!cancelled) setBusy(false)
      })
    return () => {
      cancelled = true
    }
  }, [])

  async function runWithSsh() {
    const user = username.trim()
    if (!user) return
    setBusy(true)
    setError(null)
    try {
      const scan = await scanLan({
        username: user,
        password: password || undefined,
        keyPath: keyPath.trim() || undefined,
        port: Number(port) || 22,
      })
      setResult(scan)
    } catch (err) {
      setError(String(err))
    } finally {
      setBusy(false)
    }
  }

  const devices = result?.devices ?? []
  const missingCpu = devices.some((device) => !device.cpu && device.sshReady)

  return (
    <div
      className="import-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose()
      }}
    >
      <section
        className="import-dialog scan-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Local network scan"
      >
        <header className="import-header">
          <h2 className="import-title">Local network scan</h2>
          <button
            type="button"
            className="icon-button"
            aria-label="Close network scan"
            disabled={busy}
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        <p className="detail-description">
          Sweeps the current /24 subnet for live hosts. Without SSH only the
          operating system inferred from banners is available; sign in to read
          the real CPU model and core count.
        </p>

        <div className="scan-fields">
          <label className="scan-field">
            <span>SSH username</span>
            <input
              className="popover-input"
              type="text"
              placeholder="Optional"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !busy) void runWithSsh()
              }}
            />
          </label>
          <label className="scan-field">
            <span>Password / key passphrase</span>
            <input
              className="popover-input"
              type="password"
              autoComplete="off"
              disabled={busy}
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !busy) void runWithSsh()
              }}
            />
          </label>
          <label className="scan-field">
            <span>Private key path</span>
            <input
              className="popover-input"
              type="text"
              placeholder="~/.ssh/id_ed25519"
              autoComplete="off"
              spellCheck={false}
              disabled={busy}
              value={keyPath}
              onChange={(event) => setKeyPath(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && !busy) void runWithSsh()
              }}
            />
          </label>
          <label className="scan-field scan-field--port">
            <span>Port</span>
            <input
              className="popover-input"
              type="text"
              inputMode="numeric"
              disabled={busy}
              value={port}
              onChange={(event) => setPort(event.target.value)}
            />
          </label>
        </div>

        <div className="scan-toolbar">
          <p className="scan-note">
            Credentials are used for this scan only and are never stored. Host
            keys are checked against ~/.ssh/known_hosts.
          </p>
          <button
            type="button"
            className="popover-button popover-button--primary"
            disabled={busy || !username.trim()}
            onClick={() => void runWithSsh()}
          >
            {busy ? 'Scanning…' : 'Scan with SSH'}
          </button>
        </div>

        {error && (
          <p className="popover-error" role="alert">
            {error}
          </p>
        )}

        {busy && !result && (
          <p className="scan-summary">Scanning the local network…</p>
        )}

        {result && (
          <div className="preview-section">
            <p className="scan-summary">
              {devices.length} host{devices.length === 1 ? '' : 's'} on{' '}
              {result.subnet} · {result.scanned} addresses scanned
            </p>

            {devices.length === 0 ? (
              <p className="scan-empty">
                No hosts responded. Check that the app is on the same network as
                the devices you expect.
              </p>
            ) : (
              <div className="scan-table-wrap">
                <table className="scan-table">
                  <thead>
                    <tr>
                      <th>Address</th>
                      <th>Hostname</th>
                      <th>Operating system</th>
                      <th>CPU</th>
                      <th>Cores</th>
                      <th>Ports</th>
                    </tr>
                  </thead>
                  <tbody>
                    {devices.map((device) => (
                      <tr key={device.ip} title={device.note ?? undefined}>
                        <td className="preview-mono">{device.ip}</td>
                        <td>{device.hostname ?? '—'}</td>
                        <td>
                          {device.os
                            ? `${device.os}${
                                device.arch ? ` · ${device.arch}` : ''
                              }`
                            : '—'}
                        </td>
                        <td>
                          {device.cpu ?? (
                            <span
                              className="scan-muted"
                              title={
                                device.sshReady
                                  ? 'SSH credentials are required to read the CPU model.'
                                  : undefined
                              }
                            >
                              —
                            </span>
                          )}
                        </td>
                        <td>{device.cores ?? '—'}</td>
                        <td className="preview-mono">
                          {device.ports.length ? device.ports.join(', ') : '—'}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}

            {missingCpu && (
              <p className="scan-hint">
                Some SSH hosts did not report a CPU. Enter a username and a
                password or key, then choose “Scan with SSH”.
              </p>
            )}
          </div>
        )}

        <div className="import-actions">
          <button
            type="button"
            className="popover-button"
            disabled={busy}
            onClick={onClose}
          >
            Close
          </button>
        </div>
      </section>
    </div>
  )
}