// No Shell dependencies: this is also exercised by the Node regression tests.
export function remaining(window, now) {
    if (!window || !Number.isFinite(window.utilization) || window.utilization < 0)
        return null;
    if (window.reset != null && (!Number.isFinite(window.reset) || window.reset <= now))
        return null;
    return Math.max(0, Math.min(1, 1 - window.utilization));
}
export function percent(value) {
    return value == null ? '—' : `${Math.ceil(value * 100)}%`;
}
export function derive(snapshot, now = Date.now() / 1000) {
    if (!snapshot || snapshot.protocol !== 1)
        return {online: false, title: '—', fraction: null, status: 'Приложение не запущено', profile: null};
    const profile = snapshot.profile;
    const age = now - profile?.observedAt;
    const stale = !Number.isFinite(age) || age < -5 || age >= 600;
    const live = snapshot.phase === 'listening' && !snapshot.connectionIssue;
    const fraction = live && !stale ? remaining(profile?.windows?.['5h'], now) : null;
    const status = !snapshot.setupComplete ? 'Заверши подключение в приложении'
        : snapshot.connectionIssue ? 'Проверь соединение в приложении'
        : snapshot.phase === 'starting' ? 'Запускаем перехват…'
        : snapshot.phase !== 'listening' ? 'Перехват выключен'
        : !profile ? 'Напиши сообщение Claude — появятся лимиты'
        : stale ? 'Данные устарели · ждём ответ Claude' : 'Лимиты получены';
    return {online: true, title: percent(fraction), fraction, status, profile, stale};
}
export function resetText(reset, now = Date.now() / 1000) {
    if (!Number.isFinite(reset) || reset <= 0 || reset >= 253402300800)
        return 'Время сброса неизвестно';
    if (reset <= now)
        return 'Время сброса прошло · ждём новые данные';
    const minutes = Math.max(1, Math.ceil((reset - now) / 60));
    const duration = minutes >= 1440 ? `${Math.floor(minutes / 1440)} д ${Math.floor(minutes % 1440 / 60)} ч`
        : minutes >= 60 ? `${Math.floor(minutes / 60)} ч ${minutes % 60} мин` : `${minutes} мин`;
    const date = new Date(reset * 1000).toLocaleString('ru-RU', {
        day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit',
    });
    return `Сброс ${date} · через ${duration}`;
}
