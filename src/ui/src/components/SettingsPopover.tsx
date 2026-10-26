type Props = {
  onClose: () => void
}

export function SettingsPopover({ onClose }: Props) {
  return (
    <div className="popover" role="dialog" aria-label="Settings">
      <h2 className="popover-title">Settings</h2>
      <div className="popover-actions">
        <button
          type="button"
          className="popover-button popover-button--primary"
          onClick={onClose}
        >
          Close
        </button>
      </div>
    </div>
  )
}