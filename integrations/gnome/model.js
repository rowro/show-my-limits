// No Shell dependencies: this is also exercised by the Node regression tests.
export function remaining(window, now) {
    if (!window || !Number.isFinite(window.utilization) || window.utilization < 0)
        return null;
    if (window.reset != null && (!Number.isFinite(window.reset) || window.reset <= now))
        return null;
    return Math.max(0, Math.min(1, 1 - window.utilization));
}
export function percent(value) {
    return value == null ? '—' : `${new Intl.NumberFormat('ru-RU', {maximumFractionDigits: 1}).format(value * 100)}%`;
}
export function derive(snapshot, now = Date.now() / 1000) {
    if (!snapshot || snapshot.protocol !== 1)
        return {online: false, title: '—', fraction: null, status: 'Приложение не запущено', profile: null};
    const profile = snapshot.profile;
    const age = now - profile?.observedAt;
    const stale = !Number.isFinite(age) || age < -5 || age >= 600;
    const live = snapshot.setupComplete && snapshot.phase === 'listening' && !snapshot.connectionIssue;
    const fraction = live && !stale ? remaining(profile?.windows?.['5h'], now) : null;
    const status = !snapshot.setupComplete ? 'Заверши подключение в приложении'
        : snapshot.connectionIssue ? 'Проверь соединение в приложении'
        : snapshot.phase === 'starting' ? 'Запускаем перехват…'
        : snapshot.phase !== 'listening' ? 'Перехват выключен'
        : !profile ? 'Напиши сообщение Claude — появятся лимиты'
        : stale ? 'Данные устарели · ждём ответ Claude'
        : `Обновлено ${age < 60 ? 'только что' : `${Math.floor(age / 60)} мин назад`}`;
    return {online: true, title: percent(fraction), fraction, status, profile, stale};
}
// Keep transient transport errors separate from real app/process disappearance.
export class SnapshotTracker {
    constructor() { this.reset(); }
    reset() { this.snapshot = null; this.lastSuccess = 0; this.failures = 0; }
    accept(snapshot, now = Date.now()) {
        this.snapshot = snapshot;
        this.lastSuccess = now;
        this.failures = 0;
        return snapshot;
    }
    fail(now = Date.now()) {
        this.failures++;
        return this.failures < 3 && now - this.lastSuccess < 15000 ? this.snapshot : null;
    }
}
