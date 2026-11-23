import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export type LifecycleState =
  | 'not_installed'
  | 'installing'
  | 'stopped'
  | 'starting'
  | 'running'
  | 'stopping'
  | 'failed'
  | 'uninstalling'

export type AppSummary = {
  id: string
  name: string
  description: string
  glyph: string
  color: string
  iconUrl: string | null
  draft: boolean
  state: LifecycleState
  version: string | null
  managed: boolean
  sourceUrl: string | null
  methodId: string | null
  methodKind: string | null
  /** Whether a verified long-running start command is available. */
  startable: boolean
  /** Why Start is unavailable, shown next to the start-command editor. */
  startReason: string | null
  /** Newest published version when it is newer than the installed one. */
  updateAvailable: string | null
}

export type UsageApi = {
  base_url: string | null
  openai_base_url: string | null
  docs_url: string | null
}

export type UsageCli = {
  id: string | null
  label: string
  command: string
}

export type UsageLaunch = {
  command: string
  source: string | null
  confidence: string | null
}

export type Usage = {
  ui_url: string | null
  website: string | null
  api: UsageApi | null
  docs_url: string | null
  cli: UsageCli[]
  launch: UsageLaunch | null
  inferred: boolean
  source: string | null
  confidence: string | null
  notes: string | null
}

export type UsageItemState = {
  id: string
  kind: string
  label: string
  value: string
  state: 'verified' | 'stale' | 'failed' | 'unverified'
  status: string | null
  message: string | null
  checkedAt: string | null
  details: Record<string, unknown> | null
}

export type UsageReport = {
  appId: string
  methodId: string
  version: string
  checkedAt: string | null
  items: UsageItemState[]
}

export type DetectedEntry = {
  kind: string
  url: string
  title: string | null
  confidence: string
  evidence: string
}

export type AppDetail = Omit<
  AppSummary,
  'iconUrl' | 'updateAvailable'
> & {
  installDir: string | null
  method: string | null
  healthOk: boolean
  healthMonitored: boolean
  provenanceJson: string | null
  bomPath: string | null
  usage: Usage | null
  /** User-configured command the Start action runs instead of the recipe's. */
  startCommand: string[] | null
}

export type StartCandidate = {
  command: string[]
  health:
    | { type: 'http'; url: string }
    | { type: 'exec'; command: string[] }
    | { type: 'port'; port: number }
    | { type: 'process' }
    | { type: 'none' }
  detach: 'foreground' | 'daemonize'
  confidence: string
  reason: string
  stopCommand: string[] | null
}

export type StartVerification = {
  ok: boolean
  status: number | null
  output: string
  reason: string
}

export type ExistingInstall = {
  methodId: string | null
  kind: string
  location: string
  version: string | null
  managed: boolean
}

export type InstallPlanRejection = {
  methodId: string
  reason: string
}

export type InstallPlan = {
  appId: string
  methodId: string
  methodKind: string
  tier: string
  score: number
  rationale: string[]
  rejected: InstallPlanRejection[]
  estimatedSize: number | null
  needsElevation: boolean
  advanced: boolean
  existing: ExistingInstall[]
  action: string
}

export type RecipeMethod = {
  id: string
  kind: string
  when: { os: string[]; arch: string[] }
  source: { repo: string | null; asset: Record<string, string> } | null
  service: { command: string[]; health_url?: string } | null
  manageable?: boolean | null
}

export type Recipe = {
  id: string
  name: string
  description: string
  glyph: string
  color: string
  icon: string
  draft: boolean
  platforms: string[]
  methods: RecipeMethod[]
}

export type MethodSettings = {
  order: string[]
  enabled: Record<string, boolean>
}

export type RecipePreview = {
  recipe: Recipe
  sourceUrl: string
  sha256: string
  platform: string
  supported: boolean
  detectedPlatforms: string[]
  unusable: boolean
  hint: string | null
  conflict: boolean
  warnings: string[]
}

export type BundleEntry = {
  id: string
  name: string
  version: string | null
  methodId: string | null
  status: 'install' | 'installed' | 'unsupported'
  message: string | null
}

export type UpdateStatus = {
  appId: string
  installedVersion: string
  latestVersion: string
  available: boolean
  source: string
  checkedAt: string
}

export type UpdateAvailableEvent = {
  appId: string
  installedVersion: string
  latestVersion: string
}

export type JobRecord = {
  id: number
  appId: string
  kind: string
  origin: string
  state: string
  error: string | null
  logPath: string
  createdAt: string
  finishedAt: string | null
}

export type AppStateEvent = {
  appId: string
  state: LifecycleState
  version: string | null
  managed: boolean
}

export type JobLogEvent = {
  jobId: number
  appId: string
  line: string
}

export type JobFinishedEvent = {
  jobId: number
  appId: string
  state: string
  error: string | null
}

export function listApps(): Promise<AppSummary[]> {
  return invoke('list_apps')
}

export function getAppDetail(appId: string): Promise<AppDetail> {
  return invoke('get_app_detail', { appId })
}

export function planInstall(
  appId: string,
  methodId: string | null,
  allowAdvanced: boolean,
  allowElevation: boolean,
  upgrade = false,
): Promise<InstallPlan> {
  return invoke('plan_install', {
    appId,
    methodId,
    allowAdvanced,
    allowElevation,
    upgrade,
  })
}

export function installApp(
  appId: string,
  methodId: string | null,
  allowAdvanced: boolean,
  allowElevation: boolean,
  upgrade = false,
): Promise<void> {
  return invoke('install_app', {
    appId,
    methodId,
    allowAdvanced,
    allowElevation,
    upgrade,
    origin: 'ui',
    confirmed: true,
  })
}

export function startApp(appId: string): Promise<void> {
  return invoke('start_app', { appId, origin: 'ui', confirmed: true })
}

export function stopApp(appId: string): Promise<void> {
  return invoke('stop_app', { appId, origin: 'ui', confirmed: true })
}

export function uninstallApp(appId: string): Promise<void> {
  return invoke('uninstall_app', { appId, origin: 'ui', confirmed: true })
}

export function listJobs(appId: string): Promise<JobRecord[]> {
  return invoke('list_jobs', { appId })
}

export function getJobLog(jobId: number): Promise<string> {
  return invoke('get_job_log', { jobId })
}

// Best-effort: a failure to report must not surface another error.
export function reportUiError(context: string, err: unknown): void {
  invoke('log_ui_error', { context, message: String(err) }).catch(() => {})
}

export function previewRecipe(url: string): Promise<RecipePreview> {
  return invoke('preview_recipe', { url })
}

export function importRecipe(url: string, sha256: string): Promise<void> {
  return invoke('import_recipe', {
    url,
    sha256,
    origin: 'ui',
    confirmed: true,
  })
}

export function removeApp(appId: string): Promise<void> {
  return invoke('remove_app', {
    appId,
    origin: 'ui',
    confirmed: true,
  })
}

export function saveStartCommand(
  appId: string,
  command: string[],
): Promise<void> {
  return invoke('save_start_command', {
    appId,
    command,
    origin: 'ui',
    confirmed: true,
  })
}

export type HealthCheck = StartCandidate['health']
export type DetachMode = StartCandidate['detach']

export function applyStartContract(
  appId: string,
  command: string[],
  health: HealthCheck,
  detach: DetachMode,
  stopCommand: string[] | null,
): Promise<void> {
  return invoke('apply_start_contract', {
    appId,
    command,
    health,
    detach,
    stopCommand,
    origin: 'ui',
    confirmed: true,
  })
}

export function detectStartCommands(
  appId: string,
): Promise<StartCandidate[]> {
  return invoke('detect_start_commands', {
    appId,
    origin: 'ui',
    confirmed: true,
  })
}

export function testStartCommand(
  appId: string,
  command: string[],
): Promise<StartVerification> {
  return invoke('test_start_command', {
    appId,
    command,
    origin: 'ui',
    confirmed: true,
  })
}

export function previewBundle(json: string): Promise<BundleEntry[]> {
  return invoke('preview_bundle', { json })
}

export function importBundle(json: string): Promise<BundleEntry[]> {
  return invoke('import_bundle', { json, origin: 'ui', confirmed: true })
}

export function exportBundle(): Promise<string | null> {
  return invoke('export_bundle', { origin: 'ui', confirmed: true })
}

export function openUrl(appId: string, url: string): Promise<void> {
  return invoke('open_url', {
    appId,
    url,
    origin: 'ui',
    confirmed: true,
  })
}

export function openLocalUrl(url: string): Promise<void> {
  return invoke('open_local_url', { url })
}

export function getUsageReport(appId: string): Promise<UsageReport> {
  return invoke('get_usage_report', { appId })
}

export function runUsageCheck(appId: string): Promise<UsageReport> {
  return invoke('usage_check', { appId })
}

export function discoverUsage(appId: string): Promise<DetectedEntry[]> {
  return invoke('discover_usage', { appId })
}

export function reinferUsage(appId: string): Promise<void> {
  return invoke('reinfer_usage', {
    appId,
    origin: 'ui',
    confirmed: true,
  })
}

export function getMethodSettings(): Promise<MethodSettings> {
  return invoke('get_method_settings')
}

export function saveMethodSettings(settings: MethodSettings): Promise<void> {
  return invoke('save_method_settings', {
    settings,
    origin: 'ui',
    confirmed: true,
  })
}

export type InstallRootInfo = {
  /** Directory new installs land in right now. */
  root: string
  /** The built-in `<app data>/apps` location, used when nothing is configured. */
  defaultRoot: string
  /** Raw persisted value; empty when unset. */
  configured: string
  /** True when the saved value differs from the one in effect this session. */
  restartRequired: boolean
}

export function getInstallRoot(): Promise<InstallRootInfo> {
  return invoke('get_install_root')
}

export function saveInstallRoot(path: string): Promise<InstallRootInfo> {
  return invoke('save_install_root', {
    path,
    origin: 'ui',
    confirmed: true,
  })
}

export function checkUpdates(appId: string): Promise<UpdateStatus> {
  return invoke('check_updates', { appId })
}

export function checkAllUpdates(): Promise<UpdateStatus[]> {
  return invoke('check_all_updates')
}

export function getUpdateStatus(appId: string): Promise<UpdateStatus | null> {
  return invoke('get_update_status', { appId })
}

export function onUpdateAvailable(
  handler: (event: UpdateAvailableEvent) => void,
): Promise<UnlistenFn> {
  return listen<UpdateAvailableEvent>('app-update-available', (event) =>
    handler(event.payload),
  )
}

export function onAppState(
  handler: (event: AppStateEvent) => void,
): Promise<UnlistenFn> {
  return listen<AppStateEvent>('app-state-changed', (event) =>
    handler(event.payload),
  )
}

export function onJobLog(
  handler: (event: JobLogEvent) => void,
): Promise<UnlistenFn> {
  return listen<JobLogEvent>('job-log', (event) => handler(event.payload))
}

export function onJobFinished(
  handler: (event: JobFinishedEvent) => void,
): Promise<UnlistenFn> {
  return listen<JobFinishedEvent>('job-finished', (event) =>
    handler(event.payload),
  )
}
export type LanDevice = {
  ip: string
  hostname: string | null
  os: string | null
  arch: string | null
  cpu: string | null
  cores: number | null
  ports: number[]
  sshReady: boolean
  note: string | null
}

export type LanScanResult = {
  subnet: string
  scanned: number
  devices: LanDevice[]
}

export type SshCredentials = {
  username: string
  password?: string
  keyPath?: string
  port?: number
}

/** Sweep the current /24. Credentials are optional; they unlock CPU details. */
export function scanLan(
  credentials: SshCredentials | null,
): Promise<LanScanResult> {
  return invoke('scan_lan', { credentials })
}
