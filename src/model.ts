export type Theme = "system" | "light" | "dark";
export interface Settings { theme: Theme; hosts: string[]; clients: string[]; additionalProcesses: string[]; selectedProfile: string | null; helperPath: string; certificateTrustConfirmed: boolean }
export interface QuotaWindow { utilization?: number | null; reset?: number | null; status?: string | null }
export interface Profile { id: string; host: string; observedAt: number; windows: Record<string, QuotaWindow>; status?: string | null; overageReason?: string | null }
export interface Status {
  settings: Settings;
  profiles: Profile[];
  capture: { phase: "stopped" | "starting" | "listening" | "stopping"; pid: number | null; startedAt: number | null; error: string | null; observation: string | null; approval?: string | null };
  setup: { platform: string; helperPath: string | null; certificatePath: string; certificateExists: boolean; certificateTrusted: boolean | null; certificateTrustError: string | null; certificateSecurityIssue: string | null; certificateTrustScope: string | null; certificateName: string | null; certificateSha256: string | null; certificateExpiresAt: number | null; certificateFormat: "leaf" | "legacy" | null; certificateHosts: string[]; certificateReplaceable: boolean; machineTrustCommand?: string | null; installSupported: boolean; launchAtLogin: boolean | null; installation: { phase: string; downloaded: number; total: number | null; error: string | null } };
  rotation: { phase: string; pendingSha256: string | null; pendingCertificatePath: string | null; pendingTrusted: boolean | null; pendingIssue: string | null; pendingHosts: string[]; previousSha256: string | null; previousCertificatePath: string | null; previousTrustScope: string | null; trialReady: boolean; automatic: boolean; recommended: boolean; trialOptional: boolean };
  connectionIssue: string | null;
  now: number;
}

export function certificateTrusted(state: Status): boolean {
  return state.setup.certificateTrusted ?? (state.settings.certificateTrustConfirmed || state.profiles.length > 0);
}

export function needsSetup(state: Status): boolean {
  return !state.setup.helperPath || !state.setup.certificateExists || !state.settings.hosts.length ||
    !certificateTrusted(state);
}

/** Servers selected in settings that the active leaf-only CA does not cover. */
export function uncoveredHosts(state: Status): string[] {
  if (state.setup.certificateFormat !== "leaf") return [];
  return state.settings.hosts.filter(host => !state.setup.certificateHosts.includes(host.trim().toLowerCase()));
}

export function needsClientRestart(state: Status): boolean {
  const started = state.capture.startedAt;
  return state.capture.phase === "listening" && started != null &&
    !state.profiles.some(profile => profile.observedAt >= started);
}

export function remaining(window: QuotaWindow | undefined, now: number): number | null {
  const used = window?.utilization;
  if (used == null || !Number.isFinite(used) || used < 0) return null;
  if (window?.reset != null && (!Number.isFinite(window.reset) || window.reset <= now)) return null;
  return Math.max(0, 1 - used);
}

export function isFresh(profile: Profile | undefined, now: number): boolean {
  return !!profile && now >= profile.observedAt - 5 && now - profile.observedAt < 600;
}

export function percentage(value: number): string {
  return new Intl.NumberFormat("ru-RU", { maximumFractionDigits: 1 }).format(value * 100) + "%";
}

export function age(seconds: number): string {
  if (seconds < 5) return "только что";
  if (seconds < 60) return `${Math.floor(seconds)} сек назад`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} мин назад`;
  return `${Math.floor(seconds / 3600)} ч назад`;
}

export function countdown(reset: number, now: number): string {
  if (reset <= now) return "Ждём новый ответ Claude";
  const minutes = Math.max(1, Math.ceil((reset - now) / 60));
  if (minutes >= 1440) return `через ${Math.floor(minutes / 1440)} д ${Math.floor((minutes % 1440) / 60)} ч`;
  if (minutes >= 60) return `через ${Math.floor(minutes / 60)} ч ${minutes % 60} мин`;
  return `через ${minutes} мин`;
}

export function demoStatus(): Status {
  const now = Date.now() / 1000;
  return { settings: { theme: "system", hosts: ["gateway.example"], clients: ["desktop", "terminal"], additionalProcesses: [], selectedProfile: null, helperPath: "", certificateTrustConfirmed: true },
    profiles: [{ id: "demo", host: "gateway.example", observedAt: now - 18, status: "allowed", windows: {
      "5h": { utilization: 0.27, reset: now + 8100 }, "7d": { utilization: 0.58, reset: now + 259200 }
    }}], capture: { phase: "listening", pid: null, startedAt: now - 100, error: null, observation: "quota_received" },
    setup: { platform: "macos", helperPath: "/example/mitmdump", certificatePath: "/example/show-my-limits-ca.cer", certificateExists: true, certificateTrusted: true, certificateTrustError: null, certificateSecurityIssue: null, certificateTrustScope: null, certificateName: "Show My Limits CA", certificateSha256: null, certificateExpiresAt: null, certificateFormat: "leaf", certificateHosts: ["gateway.example"], certificateReplaceable: false, installSupported: true, launchAtLogin: false, installation: { phase: "idle", downloaded: 0, total: null, error: null } },
    rotation: { phase: "none", pendingSha256: null, pendingCertificatePath: null, pendingTrusted: null, pendingIssue: null, pendingHosts: [], previousSha256: null, previousCertificatePath: null, previousTrustScope: null, trialReady: false, trialOptional: false, automatic: false, recommended: false },
    connectionIssue: null, now };
}
