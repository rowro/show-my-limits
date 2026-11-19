import { invoke, isTauri } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { age, countdown, demoStatus, certificateTrusted, needsSetup, needsClientRestart, isFresh, percentage, remaining, type Profile, type Settings, type Status, type Theme } from "./model";
import "./styles.css";
import brandLogo from "./assets/show-my-limits.svg";

const native = isTauri();
const devTools = !native && import.meta.env.DEV;
const demo = !native && new URLSearchParams(location.search).has("demo");
const root = document.querySelector<HTMLDivElement>("#app")!;
let view: "overview" | "setup" | "settings" = "setup";
let appVersion = "0.1.0";
type UpdateStatus = { phase: string; version: string | null; downloaded: number; total: number | null; error: string | null; manualInstall: boolean };
let updateState: UpdateStatus = { phase: "idle", version: null, downloaded: 0, total: null, error: null, manualInstall: false };
let firstStatus = true;
let setupStep = 0;
let setupDraft: { hosts: string[]; additionalProcesses: string[] } | null = null;
let settingsDraft: { hosts: string[]; additionalProcesses: string[] } | null = null;
let busy = false;
let pendingAction = "";
let toast: string | null = null;
let renderedView = "";
let renderedMarkup = "";
let state: Status = { settings: { theme: "system", hosts: [], clients: ["desktop", "terminal"], additionalProcesses: [], selectedProfile: null, helperPath: "", certificateTrustConfirmed: false },
  profiles: [], capture: { phase: "stopped", pid: null, startedAt: null, error: null },
  setup: { platform: "macos", helperPath: null, certificatePath: "", certificateExists: false, certificateTrusted: false, certificateTrustError: null, certificateName: null, certificateSha256: null, installSupported: true, launchAtLogin: false, installation: { phase: "idle", downloaded: 0, total: null, error: null } }, connectionIssue: null, now: Date.now() / 1000 };
if (demo) { state = demoStatus(); view = "overview";
  if (new URLSearchParams(location.search).get("demo") === "setup") {
    state.settings.hosts = []; state.profiles = []; state.capture.phase = "stopped"; state.capture.startedAt = null; view = "setup";
  }
  if (new URLSearchParams(location.search).get("demo") === "wizard") {
    state.settings.hosts = []; state.profiles = []; state.capture.phase = "stopped"; state.capture.startedAt = null;
    state.setup.helperPath = null; state.setup.certificateExists = false; state.settings.certificateTrustConfirmed = false; state.setup.certificateTrusted = false; view = "setup";
  }
  if (new URLSearchParams(location.search).get("demo") === "restart") {
    state.profiles = []; state.capture.startedAt = state.now;
  }
}
if (view === "setup") setupStep = !state.setup.helperPath ? 0 : !state.setup.certificateExists ? 1 : !certificateTrusted(state) ? 2 : 3;
const previewTheme = new URLSearchParams(location.search).get("theme");
if (demo && ["light", "dark", "system"].includes(previewTheme || "")) state.settings.theme = previewTheme as Theme;

const icons: Record<string, string> = {
  system: '<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4"/>',
  light: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5"/>',
  dark: '<path d="M20.5 13a8.5 8.5 0 0 1-9.5-9.5A8.5 8.5 0 1 0 20.5 13Z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  trash: '<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7"/>',
  close: '<path d="m6 6 12 12M18 6 6 18"/>',
  settings: "<path d=\"M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915\" />\n  <circle cx=\"12\" cy=\"12\" r=\"3\" />",
  home: '<path d="m3 10 9-7 9 7v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1Z"/><path d="M9 21v-8h6v8"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3 2"/>',
  refresh: '<path d="M20 10a8 8 0 1 0-2 8M20 4v6h-6"/>',
  shield: '<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z"/><path d="m8 12 3 3 5-6"/>',
  check: '<path d="m5 12 4 4L19 6"/>',
  arrow: '<path d="M5 12h14m-5-5 5 5-5 5"/>',
  warning: '<circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 3v.5"/>',
};
const icon = (name: string, cls = "") => `<svg class="icon ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] || icons.clock}</svg>`;
const esc = (value: unknown) => String(value ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  localStorage.setItem("show-my-limits-theme", theme);
}
const cached = localStorage.getItem("show-my-limits-theme");
if (["system", "light", "dark"].includes(cached || "")) applyTheme(cached as Theme);
if (demo) applyTheme(state.settings.theme);

function profile(): Profile | undefined { return state.profiles.find(p => p.id === state.settings.selectedProfile) || state.profiles[0]; }
function captureActive() { return ["starting", "listening", "stopping"].includes(state.capture.phase); }
function confirmed() { return state.capture.phase === "listening" && isFresh(profile(), state.now) && !state.connectionIssue; }

function shell() {
  root.innerHTML = `<div class="app-shell">
    <header><div class="brand"><img class="brand-mark" src="${brandLogo}" alt=""/><div><h1>Show My Limits</h1><div id="header-status" class="header-status"></div></div></div>
      <div class="header-actions"><button class="icon-button settings-button" data-view="settings" title="Настройки" aria-label="Настройки" aria-pressed="false">${icon("settings")}</button></div></header>
    <main id="content"><div id="content-inner"></div></main><div id="screen-actions"></div><div id="notice" role="status" aria-live="polite"></div><footer id="footer"></footer>
  </div>`;
  render();
}

function render() {
  if (needsSetup(state)) {
    const required = !state.setup.helperPath ? 0 : !state.setup.certificateExists ? 1 : !certificateTrusted(state) ? 2 : 3;
    if (view !== "setup" || setupStep > required) setupStep = required;
    view = "setup";
  }
  document.querySelector(".header-actions")?.toggleAttribute("hidden", view === "setup");
  document.querySelector(".settings-button")?.setAttribute("aria-pressed", String(view === "settings"));
  const content = document.querySelector<HTMLElement>("#content")!;
  content.classList.toggle("wizard-content", view === "setup");
  const markup = view === "overview" ? overview() : view === "setup" ? setup() : settings();
  if (markup !== renderedMarkup || view !== renderedView) {
    const scroll = view === renderedView ? content.scrollTop : 0;
    document.querySelector("#content-inner")!.innerHTML = markup;
    content.scrollTop = scroll;
    renderedView = view; renderedMarkup = markup;
  }
  renderFooter();
}

let sizeFrame = 0;
let lastDesiredHeight = 0;
let measuring = false;
function schedulePanelSize() {
  cancelAnimationFrame(sizeFrame);
  sizeFrame = requestAnimationFrame(() => { void fitPanel(); });
}
async function fitPanel() {
  const active = document.activeElement;
  if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement || measuring) return;
  const shell = document.querySelector<HTMLElement>(".app-shell");
  const inner = document.querySelector<HTMLElement>("#content-inner");
  if (!shell || !inner) return;
  const styles = getComputedStyle(shell);
  const blockHeight = (selector: string) => {
    const element = document.querySelector<HTMLElement>(selector)!;
    const css = getComputedStyle(element);
    return element.getBoundingClientRect().height + parseFloat(css.marginTop) + parseFloat(css.marginBottom);
  };
  const desired = Math.ceil(inner.getBoundingClientRect().height + blockHeight("header") + blockHeight("#screen-actions") + blockHeight("#notice") + blockHeight("#footer") +
    parseFloat(styles.paddingTop) + parseFloat(styles.paddingBottom) + parseFloat(styles.borderTopWidth) + parseFloat(styles.borderBottomWidth) + 4);
  if (Math.abs(desired - lastDesiredHeight) < 2) return;
  lastDesiredHeight = desired;
  measuring = true;
  try {
    const height = native ? await invoke<number>("resize_popover", { height: desired }) : Math.min(Math.max(240, desired), Math.max(160, window.innerHeight - (devTools && window.innerWidth < 900 ? 120 : 40)));
    shell.style.setProperty("--panel-height", `${height}px`);
  } catch (error) { lastDesiredHeight = 0; console.warn("Не удалось изменить размер окна", error); }
  finally { measuring = false; }
}

function windowCard(key: string, label: string, item: Profile) {
  const window = item.windows[key];
  const left = remaining(window, state.now);
  const used = window?.utilization;
  const validUsed = used != null && Number.isFinite(used) && used >= 0;
  const tint = left == null ? "unknown" : left <= 0.1 ? "critical" : left <= 0.3 ? "warning" : "normal";
  const reset = window?.reset;
  const hasReset = reset != null && Number.isFinite(reset) && reset > 0 && reset < 253402300800;
  const date = hasReset ? new Intl.DateTimeFormat("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(new Date(reset * 1000)) : "неизвестен";
  return `<section class="quota-card ${tint}" aria-label="Лимит за ${label}"><div class="quota-heading"><h2>${label}</h2><div class="quota-value">${left == null ? "—" : percentage(left)}<span>осталось</span></div></div>
    <div class="progress" role="progressbar" aria-label="Остаток за ${label}" ${left != null ? `aria-valuenow="${Math.round(left * 100)}"` : ""} aria-valuemin="0" aria-valuemax="100"><i style="width:${left == null ? 0 : Math.min(100, left * 100)}%"></i></div>
    <div class="spent"><span>Использовано</span><span>${validUsed ? percentage(used) : "—"}</span></div>
    <div class="reset">${icon("clock")}<span>${hasReset ? `Сброс ${esc(date)} · ${countdown(reset, state.now)}` : "Время сброса пока неизвестно"}</span></div></section>`;
}

function restartView() {
  const hasAdditionalProcesses = state.settings.additionalProcesses.length > 0;
  const restart = [
    state.setup.platform === "macos" ? "Claude Desktop — выйди через ⌘Q и открой снова." : "Claude Desktop — полностью закрой и открой снова.",
    "Терминал — заверши Claude Code и запусти снова.",
    hasAdditionalProcesses ? "Дополнительный клиент — полностью перезапусти приложение." : "",
  ].filter(Boolean);
  return `<section class="quick-start"><h2>Быстрый старт</h2><p class="quick-start-intro">Отправь сообщение, чтобы увидеть лимиты.</p><ol class="quick-start-steps">
    <li><div><h3>Перезапусти свой клиент</h3><p>${restart.map(text => `<span>${text}</span>`).join("")}</p></div></li>
    <li><div><h3>Напиши что-нибудь в чат</h3></div></li>
    <li><div><h3>Дождись ответа Claude</h3><p>После ответа лимиты появятся здесь.</p></div></li>
  </ol></section>`;
}

function overview() {
  const item = profile();
  if (needsSetup(state)) return `<div class="empty"><img class="empty-logo" src="${brandLogo}" alt=""/><h2>Сначала подключим Claude</h2><p>Подготовь перехватчик, сертификат<br>и выбери сервер. Затем появятся лимиты.</p><button class="primary" data-view="setup">Перейти к подключению${icon("arrow")}</button></div>`;
  if (needsClientRestart(state)) return restartView();
  if (!item) return `<div class="empty"><img class="empty-logo" src="${brandLogo}" alt=""/><h2>${captureActive() ? "Ждём лимиты" : "Всё готово к подключению"}</h2><p>${captureActive() ? "Отправь сообщение в Claude.<br>Показатели появятся с новым ответом." : "Запусти захват. Затем полностью перезапусти Claude<br>и отправь сообщение."}</p>${!captureActive() ? `<button class="primary" data-action="start" ${busy ? "disabled" : ""}>Запустить захват</button>` : ""}</div>`;
  const picker = `<span class="host-name" title="${esc(item.host)}">${esc(item.host)}</span>`;
  return `<div class="profile-row">${picker}<span class="last-updated">Обновлено ${age(Math.max(0, state.now - item.observedAt))}</span></div>
    ${windowCard("5h", "5 часов", item)}${windowCard("7d", "7 дней", item)}
    ${!isFresh(item, state.now) ? '<p class="hint stale">Показатели устарели. Ждём новый ответ Claude.</p>' : ""}
    ${item.status === "rejected" ? '<p class="inline-warning">Сервер сообщает о достижении лимита.</p>' : ""}`;
}

function setup() {
  const ready = state.setup.certificateExists;
  const helper = !!state.setup.helperPath;
  const trusted = certificateTrusted(state);
  const configured = state.settings.hosts.length > 0;
  const installation = state.setup.installation;
  const installing = ["downloading", "verifying", "extracting"].includes(installation.phase);
  const progress = installation.phase === "verifying" ? "Проверяем загрузку…" : installation.phase === "extracting" ? "Устанавливаем…" : installation.total ? `Загружаем ${Math.floor(installation.downloaded / installation.total * 100)}%` : "Загружаем…";
  const name = state.setup.certificateName;
  const identity = ready ? `<div class="certificate-identity"><p>В списке сертификатов: <strong>${esc(name || "имя не удалось прочитать")}</strong></p><details><summary>Как отличить от других сертификатов</summary><p>Файл: <code>mitmproxy-ca-cert.cer</code></p><p>Если сертификатов с таким именем несколько, сравни SHA-256 в окне сведений:</p><code class="fingerprint">${esc(state.setup.certificateSha256 || "Отпечаток не удалось прочитать")}</code></details></div>` : "";
  const trust = state.setup.platform === "macos"
    ? `<ol class="trust-steps"><li>Нажми «Импортировать сертификат» ниже. В окне добавления выбери связку <strong>«Система»</strong> → «Добавить». macOS может запросить пароль.</li><li>В «Связке ключей» выбери слева <strong>«Система»</strong>, сверху — <strong>«Сертификаты»</strong>. Если слева нет списка связок: «Вид» → «Показать связки ключей».</li><li>В поиске введи <strong>${esc(name || "limits-mitmproxy")}</strong>. Ищи во вкладке «Сертификаты», а не «Пароли» или «Мои сертификаты».</li><li>Открой найденный сертификат двойным щелчком → «Доверие» → «При использовании этого сертификата» → <strong>«Всегда доверять»</strong>. Закрой окно и подтверди изменение.</li><li>Полностью перезапусти Claude, затем нажми «Я добавил доверие».</li></ol><p class="field-help">Не появился диалог импорта? В «Связке ключей» выбери «Файл» → «Импортировать объекты…» и укажи файл из кнопки «Показать файл».</p>`
    : state.setup.platform === "windows"
      ? "В мастере импорта выбери «Текущий пользователь» → «Доверенные корневые центры сертификации». Затем перезапусти клиент."
      : "Добавь сертификат в доверенные CA своего дистрибутива. У некоторых клиентов и браузеров отдельное хранилище доверия.";
  const steps = [
    `<section class="setup-step"><span class="step-number ${helper ? "done" : ""}">${helper ? icon("check") : "1"}</span><div><h3>Перехватчик <span class="pill ${helper ? "good" : ""}">${helper ? "Установлен" : "Нужен mitmproxy"}</span></h3>
      ${helper ? `<p>Найдена установленная версия. Повторная установка не нужна.</p><details class="helper-location"><summary>Где установлен</summary><p class="helper-path">${esc(state.setup.helperPath)}</p></details>` : `<p>Установим официальный mitmproxy в каталог приложения. Python и Homebrew не нужны.</p><button class="secondary" data-action="install-helper" ${busy || installing || !state.setup.installSupported ? "disabled" : ""}>${installing ? progress : "Установить mitmproxy"}</button>${!state.setup.installSupported ? '<p class="field-help">Для этой архитектуры укажи свою установку в настройках.</p>' : ""}<details><summary>Уже установлен? Указать путь</summary><form id="helper-form"><input name="helperPath" aria-label="Путь к mitmdump" placeholder="Путь к mitmdump"/><button class="secondary setup-link" type="submit">Использовать</button></form></details>`}
    </div></section>`,
    `<section class="setup-step"><span class="step-number ${ready ? "done" : ""}">${ready ? icon("check") : "2"}</span><div><h3>Сертификат <span class="pill ${ready ? "good" : ""}">${ready ? "Создан" : "Не создан"}</span></h3><p>Локальный, отдельный для этого приложения.</p><button class="secondary" data-action="certificate" ${!helper || ready || busy ? "disabled" : ""}>${ready ? "Сертификат готов" : busy && !installing ? "Подготавливаем…" : "Создать сертификат"}</button></div></section>`,
    `<section class="setup-step"><span class="step-number ${trusted ? "done" : ""}">${trusted ? icon("check") : "3"}</span><div><h3>Доверие сертификату <span class="pill ${trusted ? "good" : ""}">${trusted ? "Готово" : "Нужна установка"}</span></h3>
    ${state.setup.platform === "macos" ? `<p>${trusted ? "Доверие настроено. После включения перехватчика полностью выйди из Claude через ⌘Q и открой его заново." : "Добавим сертификат в связку «Система» и настроим доверие для HTTPS. macOS может запросить два подтверждения: для импорта и для доверия. Пароль вводится в системных окнах."}</p><button class="primary" data-action="install-certificate" ${!ready || busy || trusted ? "disabled" : ""}>${icon("shield")}${busy && pendingAction === "install-certificate" ? "Ожидаем разрешения…" : trusted ? "Доверие настроено" : "Установить сертификат"}</button>` : ""}
    ${identity}
    ${state.setup.platform === "macos" ? `<details class="manual-certificate"><summary>Установить вручную</summary><div class="trust-guide">${trust}</div><button class="secondary" data-action="open-certificate" ${!ready || busy ? "disabled" : ""}>${icon("shield")}${state.setup.platform === "macos" ? "Импортировать сертификат" : "Открыть сертификат"}</button><button class="text-button setup-link" data-action="reveal-certificate" ${!ready || busy ? "disabled" : ""}>Показать файл</button>${!trusted ? `<button class="text-button setup-link" data-action="confirm-trust" ${!ready || busy ? "disabled" : ""}>Я добавил доверие</button>` : ""}</details>` : `<div class="trust-guide">${trust}</div><button class="secondary" data-action="open-certificate" ${!ready || busy ? "disabled" : ""}>${icon("shield")}${state.setup.platform === "macos" ? "Импортировать сертификат" : "Открыть сертификат"}</button><button class="text-button setup-link" data-action="reveal-certificate" ${!ready || busy ? "disabled" : ""}>Показать файл</button>${!trusted ? `<button class="text-button setup-link" data-action="confirm-trust" ${!ready || busy ? "disabled" : ""}>Я добавил доверие</button>` : ""}`}
    <p class="subtle">Получение нового ответа Claude подтвердит, что клиент доверяет сертификату.</p></div></section>`,
    `<section class="setup-step"><span class="step-number ${configured ? "done" : ""}">${configured ? icon("check") : "4"}</span><div><h3>Сервер и процессы <span class="pill ${configured ? "good" : ""}">${configured ? "Выбраны" : "Не выбраны"}</span></h3><form id="setup-form">${connectionFields({ ...state.settings, ...(setupDraft || {}) })}</form></div></section>`
  ];
  return `<div class="wizard-heading"><span class="wizard-caption">Подключение · шаг ${setupStep + 1} из 4</span><div class="wizard-progress" aria-label="Шаг ${setupStep + 1} из 4">${steps.map((_, i) => `<i class="${i <= setupStep ? "active" : ""}"></i>`).join("")}</div></div>${steps[setupStep]}`;

}

function connectionFields(s: Settings) {
  return `<fieldset><legend><label for="server">Сервер</label></legend><input type="text" id="server" name="hosts" value="${esc(s.hosts[0] || "")}" spellcheck="false" placeholder="api.anthropic.com"/><p class="field-help">Домен или URL. Путь запросов может быть любым.</p></fieldset>
    <fieldset><legend>Процессы</legend><p class="process-defaults">По умолчанию перехватываем <code>Claude</code> и <code>claude</code>.</p><label class="field-label" for="additional-processes">Дополнительные процессы</label><input type="text" id="additional-processes" name="additionalProcesses" value="${esc(s.additionalProcesses.join(", "))}" placeholder="Code, extensionHost" spellcheck="false"/><p class="field-help">Имена процессов через запятую, с учётом регистра. Например, для конкретного расширения IDE.</p></fieldset>`;
}

function settings() {
  const s = state.settings;
  return `<form id="settings-form"><div class="section-heading"><h2>Настройки</h2></div>
    <fieldset><legend>Оформление</legend><div class="theme-switch" role="group" aria-label="Тема">${([['system','Система'],['light','Светлая'],['dark','Тёмная']] as const).map(([value, label]) => `<button type="button" data-theme="${value}" class="${s.theme === value ? "selected" : ""}" aria-label="${label}" title="${label}" aria-pressed="${s.theme === value}">${icon(value)}</button>`).join("")}</div></fieldset>
    <fieldset><legend>Запуск</legend><label class="client-option"><input type="checkbox" id="launch-at-login" ${state.setup.launchAtLogin ? "checked" : ""} ${state.setup.launchAtLogin == null ? "disabled" : ""}/><span><strong>Запускать вместе с ОС</strong><small>В фоне, со значком в трее.</small></span></label><p class="field-help">${state.setup.launchAtLogin == null ? "Не удалось прочитать настройку автозапуска." : state.setup.platform === "macos" ? "Для автозапуска держи приложение в «Программах»." : "Настройка применяется сразу."}</p></fieldset>
    <section id="updates-section"></section>
    ${connectionFields({ ...s, ...(settingsDraft || {}) })}
    <p class="field-help">Если изменить сервер или процессы, захват перезапустится при сохранении.</p></form>`;
}

function setupStepReady() {
  return [!!state.setup.helperPath, state.setup.certificateExists,
    certificateTrusted(state), true][setupStep];
}

function renderUpdates() {
  const section=document.querySelector("#updates-section"); if (!section) return;
  const phase=updateState.phase;
  const working=["checking", "downloading", "installing"].includes(phase);
  const text=phase === "checking" ? "Проверяем…" : phase === "downloading" ? `Загружаем${updateState.total ? ` ${Math.floor(updateState.downloaded / updateState.total * 100)}%` : "…"}` : phase === "installing" ? "Устанавливаем…" : phase === "available" ? `Доступна версия ${updateState.version}` : phase === "current" ? "Установлена актуальная версия" : phase === "unpublished" ? "Обновления пока не опубликованы" : phase === "error" ? updateState.error : "Новые версии из GitHub Releases";
  section.innerHTML=`<fieldset><legend>Обновления</legend><p class="field-help">${esc(text)}</p>${phase === "available" ? `<button type="button" class="secondary update-button" data-action="${updateState.manualInstall ? "open-releases" : "install-update"}">${updateState.manualInstall ? "Скачать пакет" : "Установить и перезапустить"}</button>` : `<button type="button" class="secondary update-button" data-action="check-update" ${working ? "disabled" : ""}>Проверить обновления</button>`}</fieldset>`;
}
function renderFooter() {
  renderUpdates();
  const phase = state.capture.phase;
  const text = phase === "stopped" ? "Перехват выключен" : phase === "starting" ? "Запускаем захват…" : phase === "stopping" ? "Останавливаем…" : state.setup.certificateExists && state.setup.certificateTrusted === false ? "Нужно доверие к CA" : state.connectionIssue ? "Сбой TLS-соединения" : confirmed() ? "Лимиты получены" : "Ждём ответ Claude";
  document.querySelector("#header-status")!.innerHTML = `<span class="capture-status"><i class="status-dot ${confirmed() ? "green" : phase !== "stopped" ? "amber" : ""}"></i>${text}</span>${needsSetup(state) && phase === "stopped" ? '<button class="capture-button start" data-view="setup">Подключить</button>' : `<button class="capture-button ${phase === "stopped" ? "start" : ""}" data-action="${phase === "stopped" ? "start" : "stop"}" ${busy || ["starting", "stopping"].includes(phase) ? "disabled" : ""}>${phase === "stopped" ? "Запустить" : "Остановить"}</button>`}`;
  const wizardNav = view === "setup" ? `<div class="wizard-nav"><button class="secondary" data-action="setup-back" ${busy || setupStep === 0 ? "disabled" : ""}>Назад</button>${setupStep === 3 ? `<button class="primary" type="submit" form="setup-form" ${busy ? "disabled" : ""}>${busy ? "Подключаем…" : "Подключить"}</button>` : `<button class="primary" data-action="setup-next" ${busy || !setupStepReady() ? "disabled" : ""}>Далее${icon("arrow")}</button>`}</div>` : "";
  document.querySelector("#screen-actions")!.innerHTML = `${wizardNav}${view === "settings" ? `<button class="primary screen-save" type="submit" form="settings-form" ${busy ? "disabled" : ""}>${busy ? "Сохраняем…" : "Сохранить"}</button>` : ""}`;
  document.querySelector("#footer")!.innerHTML = `<div class="footer-bottom"><span class="version">${esc(appVersion)}</span><button class="text-button" data-action="quit">Выйти</button></div>`;
  const message = toast || state.setup.certificateTrustError || state.setup.installation.error || state.capture.error || state.connectionIssue;
  const notice = document.querySelector("#notice")!;
  notice.innerHTML = message ? `<div class="notice">${icon("warning")}<span>${esc(message)}</span><button class="icon-button" data-action="dismiss" aria-label="Закрыть сообщение">${icon("close")}</button></div>` : "";
}

async function refresh() {
  if (!native) return;
  try { state = await invoke<Status>("get_status"); if (firstStatus) { view = needsSetup(state) ? "setup" : "overview"; setupStep = !state.setup.helperPath ? 0 : !state.setup.certificateExists ? 1 : !certificateTrusted(state) ? 2 : 3; firstStatus = false; } applyTheme(state.settings.theme); if (view !== "settings" || needsSetup(state)) render(); else renderFooter(); }
  catch { toast = "Не удалось получить состояние Show My Limits."; renderFooter(); }
}

async function finishSetup() {
  if (needsSetup(state)) return;
  if (native) await invoke("complete_setup");
  else { state.capture.phase = "starting"; state.capture.startedAt = state.now; }
  view = "overview";
  await refresh(); render();
}

async function action(name: string) {
  if (["check-update", "install-update", "open-releases"].includes(name)) {
    if (!native) { updateState.phase=name === "check-update" ? "current" : "idle"; renderUpdates(); return; }
    try {
      if (name === "check-update") updateState=await invoke<UpdateStatus>("check_update");
      if (name === "install-update") await invoke("install_update");
      if (name === "open-releases") await invoke("open_releases");
    } catch (error) { updateState.phase="error"; updateState.error=String(error); }
    renderUpdates(); return;
  }
  if (name === "dismiss") { toast = null; state.capture.error = null; state.connectionIssue = null; renderFooter(); return; }
  if (name === "setup-next" && !busy && setupStepReady()) { setupStep = Math.min(3, setupStep + 1); render(); return; }
  if (name === "setup-back" && !busy) { setupStep = Math.max(0, setupStep - 1); render(); return; }
  if (!native) {
    if (name === "install-helper") state.setup.helperPath = "/preview/mitmdump";
    if (name === "certificate") state.setup.certificateExists = true;
    if (["install-certificate", "confirm-trust"].includes(name)) { state.settings.certificateTrustConfirmed = true; state.setup.certificateTrusted = true; }
    if (name === "start") { state.capture.phase = "starting"; state.capture.startedAt = state.now; }
    if (name === "stop") state.capture.phase = "stopped";
    render(); return;
  }
  if (name === "quit") { await invoke("quit_app"); return; }
  if (busy) return;
  busy = true; pendingAction = name; toast = null; if (view === "setup") render(); else renderFooter();
  try {
    if (name === "finish-setup") await finishSetup();
    if (name === "install-helper") await invoke("install_helper");
    if (name === "confirm-trust") {
      const next = { ...state.settings, certificateTrustConfirmed: true };
      await invoke("save_settings", { settings: next }); state.settings = next;
    }
    if (name === "start") { await invoke("start_capture"); view = "overview"; }
    if (name === "stop") await invoke("stop_capture");
    if (name === "certificate") await invoke("prepare_certificate");
    if (name === "install-certificate") await invoke("install_certificate");
    if (name === "open-certificate") await invoke("open_certificate");
    if (name === "reveal-certificate") await invoke("reveal_certificate");
    await refresh();
    if (view === "setup") render();
  } catch (error) { toast = String(error); }
  finally { busy = false; pendingAction = ""; if (view === "setup") render(); else renderFooter(); }
}

root.addEventListener("click", async event => {
  const target = (event.target as HTMLElement).closest<HTMLElement>("[data-view],[data-action],[data-theme]");
  if (!target) return;
  if (target.dataset.view) {
    view = needsSetup(state) ? "setup" : target.dataset.view === "settings" && view !== "settings" ? "settings" : "overview";
    toast = null; render();
  }
  if (target.dataset.action) await action(target.dataset.action);
  if (target.dataset.theme && !busy) {
    const theme = target.dataset.theme as Theme;
    const next = { ...state.settings, theme };
    try { if (native) await invoke("save_settings", { settings: next }); state.settings = next; applyTheme(theme); document.querySelectorAll("[data-theme]").forEach(el => { const selected = (el as HTMLElement).dataset.theme === theme; el.classList.toggle("selected", selected); el.setAttribute("aria-pressed", String(selected)); }); }
    catch (error) { toast = String(error); renderFooter(); }
  }
});

root.addEventListener("submit", async event => {
  event.preventDefault();
  const form = event.target as HTMLFormElement;
  if (!["settings-form", "setup-form", "helper-form"].includes(form.id) || busy) return;
  const data = new FormData(form);
  const next: Settings = form.id === "helper-form"
    ? { ...state.settings, helperPath: String(data.get("helperPath") || "").trim() }
    : { ...state.settings, hosts: data.getAll("hosts").map(String).map(h => h.trim()).filter(Boolean),
        additionalProcesses: readProcesses(data) };
  busy = true; toast = null; renderFooter();
  try {
    if (form.id !== "helper-form" && !next.hosts.length) throw new Error("Укажи хотя бы один сервер.");
    if (native) await invoke("save_settings", { settings: next });
    state.settings = next;
    if (!native && form.id === "helper-form") state.setup.helperPath = next.helperPath || null;
    if (native) await refresh();
    if (needsSetup(state)) view = "setup";
    else if (form.id === "setup-form") { setupDraft = null; await finishSetup(); }
    else if (form.id === "settings-form") { settingsDraft = null; view = "overview"; }
  } catch (error) { toast = String(error); }
  finally { busy = false; render(); }
});

function readProcesses(data: FormData) {
  return [...new Set(String(data.get("additionalProcesses") || "").split(",").map(name => name.trim()).filter(Boolean))];
}

function rememberConnectionDraft(form: HTMLFormElement) {
  const data = new FormData(form);
  const draft = { hosts: data.getAll("hosts").map(String), additionalProcesses: readProcesses(data) };
  if (form.id === "setup-form") setupDraft = draft;
  if (form.id === "settings-form") settingsDraft = draft;
}
root.addEventListener("input", event => {
  const form = (event.target as HTMLElement).closest<HTMLFormElement>("#setup-form, #settings-form");
  if (form) rememberConnectionDraft(form);
});

root.addEventListener("change", async event => {
  if (event.target instanceof HTMLInputElement && event.target.id === "launch-at-login") {
    const checkbox = event.target;
    const previous = !!state.setup.launchAtLogin;
    checkbox.disabled = true;
    try {
      state.setup.launchAtLogin = native ? await invoke<boolean>("set_launch_at_login", { enabled: checkbox.checked }) : checkbox.checked;
      checkbox.checked = !!state.setup.launchAtLogin;
    } catch (error) { checkbox.checked = previous; toast = String(error); renderFooter(); }
    finally { checkbox.disabled = state.setup.launchAtLogin == null; }
    return;
  }

});

const devScenes = [
  ["limits", "Лимиты"], ["low", "Лимиты заканчиваются"], ["exhausted", "Лимит исчерпан"], ["stale", "Устаревшие данные"],
  ["quickstart", "Быстрый старт"], ["trust-lost", "Доверие к CA удалено"], ["stopped", "Перехват выключен"], ["settings", "Настройки"],
  ["setup-0", "1 · Перехватчик"], ["setup-1", "2 · Сертификат"], ["setup-2", "3 · Доверие"], ["setup-3", "4 · Сервер и процессы"],
];
function setDevScene(scene: string) {
  if (!devTools) return;
  if (!devScenes.some(([id]) => id === scene)) scene = "limits";
  const theme = state.settings.theme;
  state = demoStatus(); state.settings.theme = theme;
  setupDraft = null; settingsDraft = null; toast = null; busy = false; pendingAction = "";
  view = scene === "settings" ? "settings" : "overview";
  if (scene === "trust-lost") state.setup.certificateTrusted = false;
  if (scene === "low" || scene === "exhausted") {
    state.profiles[0].windows["5h"].utilization = scene === "low" ? 0.92 : 1;
    state.profiles[0].windows["7d"].utilization = 0.84;
    if (scene === "exhausted") state.profiles[0].status = "rejected";
  }
  if (scene === "stale") state.profiles[0].observedAt = state.now - 7200;
  if (scene === "quickstart" || scene === "stopped") {
    state.profiles = [];
    state.capture.startedAt = scene === "quickstart" ? state.now : null;
    state.capture.phase = scene === "quickstart" ? "listening" : "stopped";
  }
  if (scene.startsWith("setup-")) {
    setupStep = Number(scene.slice(-1)); view = "setup";
    state.profiles = []; state.settings.hosts = [];
    state.capture.phase = "stopped"; state.capture.startedAt = null;
    state.setup.helperPath = setupStep > 0 ? "/example/mitmdump" : null;
    state.setup.certificateExists = setupStep > 1;
    state.settings.certificateTrustConfirmed = setupStep > 2; state.setup.certificateTrusted = setupStep > 2;
  }
  sessionStorage.setItem("sml-dev-scene", scene);
  const url = new URL(location.href); url.searchParams.set("screen", scene);
  history.replaceState(null, "", url);
  document.querySelector<HTMLSelectElement>("#dev-scene")!.value = scene;
  render();
}
function mountDevTools() {
  if (!devTools) return;
  const controls = document.createElement("aside"); controls.className = "dev-controls";
  controls.setAttribute("aria-label", "Экраны приложения");
  controls.innerHTML = `<label for="dev-scene">Экран</label><select id="dev-scene">${devScenes.map(([id, label]) => `<option value="${id}">${label}</option>`).join("")}</select><div class="dev-theme"><span>Тема</span><button data-dev-theme="light">Светлая</button><button data-dev-theme="dark">Тёмная</button></div>`;
  document.body.append(controls);
  controls.querySelector("select")!.addEventListener("change", event => setDevScene((event.target as HTMLSelectElement).value));
  controls.addEventListener("click", event => {
    const button = (event.target as HTMLElement).closest<HTMLElement>("[data-dev-theme]");
    if (!button) return;
    state.settings.theme = button.dataset.devTheme as Theme; applyTheme(state.settings.theme); render();
  });
  const params = new URLSearchParams(location.search);
  const existingDemo = params.get("demo");
  const scene = params.get("screen") || (existingDemo ? ({ restart: "quickstart", wizard: "setup-0", setup: "setup-3" }[existingDemo] || "limits") : sessionStorage.getItem("sml-dev-scene") || "limits");
  setDevScene(scene);
}

shell();
mountDevTools();
const panelObserver = new ResizeObserver(schedulePanelSize);
["#content-inner", "header", "#screen-actions", "#notice", "#footer"].forEach(selector => panelObserver.observe(document.querySelector(selector)!));
root.addEventListener("toggle", schedulePanelSize, true);
root.addEventListener("focusout", schedulePanelSize);
window.addEventListener("focus", () => { lastDesiredHeight = 0; schedulePanelSize(); });
window.addEventListener("resize", () => { if (!native) { lastDesiredHeight = 0; schedulePanelSize(); } });
void document.fonts.ready.then(schedulePanelSize);
if (native) {
  appVersion=await getVersion();
  updateState=await invoke<UpdateStatus>("update_status");
  await listen<UpdateStatus>("show-my-limits://update", event => { updateState=event.payload; renderUpdates(); });
  await refresh();
  await listen<Status>("show-my-limits://status", event => {
    state = event.payload; applyTheme(state.settings.theme);
    if (view !== "settings" || needsSetup(state)) render(); else renderFooter();
  });
} else {
  setInterval(() => { state.now = Date.now() / 1000; if (view === "overview") render(); }, 1000);
}
