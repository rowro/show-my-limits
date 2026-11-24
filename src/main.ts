import { invoke, isTauri } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { age, countdown, demoStatus, certificateTrusted, needsSetup, needsClientRestart, isFresh, percentage, remaining, type Profile, type Settings, type Status, type Theme } from "./model";
import { setMarkupIfChanged } from "./dom";
import { linuxTrustCommands } from "./certificates";
import "./styles.css";
import brandLogo from "./assets/show-my-limits.svg";

const native = isTauri();
const devTools = !native && import.meta.env.DEV;
const demo = !native && new URLSearchParams(location.search).has("demo");
const root = document.querySelector<HTMLDivElement>("#app")!;
let view: "overview" | "setup" | "settings" = "setup";
let appVersion = "0.2.1";
type UpdateStatus = { phase: string; version: string | null; downloaded: number; total: number | null; error: string | null; manualInstall: boolean };
let updateState: UpdateStatus = { phase: "idle", version: null, downloaded: 0, total: null, error: null, manualInstall: false };
type GnomeStatus = { supported: boolean; pending: boolean; installed: boolean; active: boolean; message: string };
let gnomeStatus: GnomeStatus | null = null;
let firstStatus = true;
let setupStep = 0;
let setupDraft: { hosts: string[]; additionalProcesses: string[] } | null = null;
let settingsDraft: { hosts: string[]; additionalProcesses: string[] } | null = null;
let busy = false;
let pendingAction = "";
let toast: string | null = null;
let setupError: string | null = null;
let supportReport: string | null = null;
let pointerActive = false;
const deferredRenders = new Set<() => void>();
function deferDuringPointer(render: () => void): boolean {
  if (!pointerActive) return false;
  deferredRenders.add(render);
  return true;
}
let renderedView = "";
let renderedMarkup = "";
let state: Status = { settings: { theme: "system", hosts: [], clients: ["desktop", "terminal"], additionalProcesses: [], selectedProfile: null, helperPath: "", certificateTrustConfirmed: false },
  profiles: [], capture: { phase: "stopped", pid: null, startedAt: null, error: null, observation: null },
  setup: { platform: "macos", helperPath: null, certificatePath: "", certificateExists: false, certificateTrusted: false, certificateTrustError: null, certificateSecurityIssue: null, certificateTrustScope: null, certificateName: null, certificateSha256: null, installSupported: true, launchAtLogin: false, installation: { phase: "idle", downloaded: 0, total: null, error: null } }, connectionIssue: null, now: Date.now() / 1000 };
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
function requiredSetupStep() {
  return !state.setup.helperPath || !state.setup.certificateExists ? 0 : !certificateTrusted(state) ? 1 : 2;
}
if (view === "setup") setupStep = requiredSetupStep();
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
function captureHint() {
  if (state.connectionIssue) return state.connectionIssue;
  if (state.capture.observation === "response_received") return "Ответ выбранного сервера получен, но в нём не было заголовков лимитов. Проверь сервер и аккаунт.";
  if (state.capture.observation === "request_seen") return "Запрос к выбранному серверу перехвачен, но ответа пока нет. Проверь сеть и TLS.";
  if (state.capture.startedAt != null && state.now - state.capture.startedAt > 30 && state.capture.phase === "listening") return "Запросов к выбранному серверу пока не видно. Проверь домен и имя процесса клиента в настройках.";
  return null;
}

function shell() {
  root.innerHTML = `<div class="app-shell">
    <header><div class="brand"><img class="brand-mark" src="${brandLogo}" alt=""/><div><h1>Show My Limits</h1><div id="header-status" class="header-status"></div></div></div>
      <div class="header-actions"><button class="icon-button settings-button" data-view="settings" title="Настройки" aria-label="Настройки" aria-pressed="false">${icon("settings")}</button></div></header>
    <main id="content"><div id="content-inner"></div></main><div id="screen-actions"></div><div id="notice" role="status" aria-live="polite"></div><footer id="footer"></footer>
  </div>`;
  render();
}

function render() {
  document.documentElement.dataset.platform = state.setup.platform;
  if (deferDuringPointer(render)) return;
  if (needsSetup(state)) {
    const required = requiredSetupStep();
    if (view !== "setup" || setupStep > required) setupStep = required;
    view = "setup";
  }
  document.querySelector(".header-actions")?.toggleAttribute("hidden", view === "setup");
  document.querySelector(".settings-button")?.setAttribute("aria-pressed", String(view === "settings"));
  const content = document.querySelector<HTMLElement>("#content")!;
  content.classList.toggle("wizard-content", view === "setup");
  const markup = view === "overview" ? `${serverContext()}${systemTrustWarning()}${overview()}` : view === "setup" ? setup() : settings();
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
  if (active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement || pointerActive || measuring) return;
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
  const hint = captureHint();
  return `<section class="quick-start"><h2>Быстрый старт</h2><p class="quick-start-intro">${esc(hint || "Отправь сообщение, чтобы увидеть лимиты.")}</p><ol class="quick-start-steps">
    <li><div><h3>Перезапусти свой клиент</h3><p>${restart.map(text => `<span>${text}</span>`).join("")}</p></div></li>
    <li><div><h3>Напиши что-нибудь в чат</h3></div></li>
    <li><div><h3>Дождись ответа Claude</h3><p>После ответа лимиты появятся здесь.</p></div></li>
  </ol></section>`;
}

function serverContext() {
  const server = state.settings.hosts[0] || "не указан";
  return `<div class="server-context"><div><span>Выбранный сервер</span><strong>${esc(server)}</strong></div><button type="button" class="text-button" data-action="edit-server">Изменить</button></div>`;
}

function systemTrustWarning(inSettings = false) {
  const scope = state.setup.certificateTrustScope;
  if (scope !== "system" && scope !== "user+system") return "";
  return `<div class="ca-scope-warning" role="note"><strong>CA найден в системном хранилище доверия</strong><p>Его закрытый ключ хранится в данных этого пользователя. Проверь отпечаток ${inSettings ? "ниже" : "в Настройках → Захват"}. Не удаляй доверие до проверки нового CA.</p></div>`;
}

function certificateTrustDetails() {
  const scope = state.setup.certificateTrustScope;
  const label = scope === "user" ? "Текущий пользователь" : scope === "system" ? "Вся система" : scope === "user+system" ? "Текущий пользователь и вся система" : scope === "none" ? "В проверенном хранилище не найден" : "Не удалось определить";
  return `<p class="field-help">Область доверия: <strong>${label}</strong></p>${systemTrustWarning(true)}<details><summary>Отпечаток действующего CA</summary><code class="fingerprint">${esc(state.setup.certificateSha256 || "Не удалось прочитать")}</code><p class="field-help">Сравни SHA-256 перед изменением доверия. Замена старого CA пока не реализована.</p></details>`;
}

function overview() {
  const item = profile();
  if (needsSetup(state)) return `<div class="empty"><img class="empty-logo" src="${brandLogo}" alt=""/><h2>Сначала подключим Claude</h2><p>Подготовь перехватчик, сертификат<br>и выбери сервер. Затем появятся лимиты.</p><button class="primary" data-view="setup">Перейти к подключению${icon("arrow")}</button></div>`;
  if (state.capture.phase === "starting") return `<div class="empty"><img class="empty-logo" src="${brandLogo}" alt=""/><h2>Запускаем перехват</h2><p>${state.setup.platform === "linux" ? "Если появится запрос прав, подтверди его. " : ""}После подтверждения готовности здесь появятся шаги для Claude.</p></div>`;
  if (needsClientRestart(state)) return restartView();
  if (!item) {
    const failed = state.capture.phase === "stopped" && !!state.capture.error;
    return `<div class="empty"><img class="empty-logo" src="${brandLogo}" alt=""/><h2>${failed ? "Захват не запущен" : captureActive() ? "Ждём лимиты" : "Всё готово к подключению"}</h2><p>${failed ? "Причина указана ниже. После успешного запуска появятся шаги для Claude." : captureActive() ? esc(captureHint() || "Отправь сообщение в Claude. Показатели появятся с новым ответом.") : "Запусти захват. Затем полностью перезапусти Claude и отправь сообщение."}</p>${!captureActive() && !failed ? `<button class="primary" data-action="start" ${busy ? "disabled" : ""}>Запустить захват</button>` : ""}</div>`;
  }
  return `<div class="profile-row"><span class="last-updated">Обновлено ${age(Math.max(0, state.now - item.observedAt))}</span></div>
    ${windowCard("5h", "5 часов", item)}${windowCard("7d", "7 дней", item)}
    ${!isFresh(item, state.now) ? '<p class="hint stale">Показатели устарели. Ждём новый ответ Claude.</p>' : ""}
    ${item.status === "rejected" ? '<p class="inline-warning">Сервер сообщает о достижении лимита.</p>' : ""}`;
}

function linuxCertificateHelp(confirm = true) {
  const ready = state.setup.certificateExists;
  return `<p>В Ubuntu / Debian выполни в терминале:</p><p class="field-help">Эти команды добавят CA в системное хранилище. Ему смогут доверять другие пользователи и приложения компьютера; храни ключ CA только у себя.</p><pre class="trust-command"><code>${esc(linuxTrustCommands(state.setup.certificatePath))}</code></pre><button type="button" class="secondary" data-action="copy-trust-commands" ${!ready ? "disabled" : ""}>Скопировать команды</button><p>После успешного выполнения ${confirm ? "нажми «Я добавил доверие» и " : ""}перезапусти клиент.</p><p class="field-help">В других дистрибутивах добавь этот публичный CA штатным способом. Некоторые клиенты используют отдельное хранилище доверия.</p>`;
}

function setup() {
  const ready = state.setup.certificateExists;
  const helper = !!state.setup.helperPath;
  const trusted = certificateTrusted(state);
  const configured = state.settings.hosts.length > 0;
  const installation = state.setup.installation;
  const installing = ["downloading", "verifying", "extracting"].includes(installation.phase);
  const showManualHelper = !busy && (!state.setup.installSupported || !!setupError || !!installation.error);
  const progress = installation.phase === "verifying" ? "Проверяем загрузку…" : installation.phase === "extracting" ? "Устанавливаем…" : installation.total ? `Загружаем ${Math.floor(installation.downloaded / installation.total * 100)}%` : "Загружаем…";
  const name = state.setup.certificateName;
  const certificateFile = state.setup.certificatePath.split(/[\\/]/).pop() || "mitmproxy-ca-cert.cer";
  const identity = ready ? `<div class="certificate-identity"><p>Имя сертификата в хранилище: <strong>${esc(name || "имя не удалось прочитать")}</strong></p><details><summary>Файл и отпечаток SHA-256</summary><p>Файл экспорта: <code>${esc(certificateFile)}</code>. Его имя отличается от имени сертификата.</p><p>Если сертификатов с таким именем несколько, сравни SHA-256 в окне сведений:</p><code class="fingerprint">${esc(state.setup.certificateSha256 || "Отпечаток не удалось прочитать")}</code></details></div>` : "";
  const trust = state.setup.platform === "macos"
    ? `<ol class="trust-steps"><li>Нажми «Импортировать сертификат» ниже. В окне добавления выбери связку <strong>«Вход»</strong> текущего пользователя → «Добавить». macOS может запросить пароль.</li><li>В «Связке ключей» выбери слева <strong>«Вход»</strong>, сверху — <strong>«Сертификаты»</strong>. Если слева нет списка связок: «Вид» → «Показать связки ключей».</li><li>В поиске введи <strong>${esc(name || "limits-mitmproxy")}</strong>. Ищи во вкладке «Сертификаты», а не «Пароли» или «Мои сертификаты».</li><li>Открой найденный сертификат двойным щелчком → «Доверие» → «При использовании этого сертификата» → <strong>«Всегда доверять»</strong>. Закрой окно и подтверди изменение.</li><li>Полностью перезапусти Claude, затем нажми «Я добавил доверие».</li></ol><p class="field-help">Не появился диалог импорта? В «Связке ключей» выбери «Файл» → «Импортировать объекты…» и укажи файл из кнопки «Показать файл».</p>`
    : state.setup.platform === "windows"
      ? "В мастере импорта выбери «Текущий пользователь» → «Доверенные корневые центры сертификации». Затем перезапусти клиент."
      : linuxCertificateHelp();
  const manualTrust = state.setup.platform === "linux"
    ? `<div class="trust-guide">${trust}</div><button class="text-button setup-link" data-action="reveal-certificate" ${!ready || busy ? "disabled" : ""}>Показать файл</button>${!trusted ? `<button class="primary setup-link confirm-trust" data-action="confirm-trust" ${!ready || busy ? "disabled" : ""}>Я добавил доверие</button>` : ""}`
    : `<details class="manual-certificate"><summary>Установить вручную</summary><div class="trust-guide">${trust}</div><button class="secondary" data-action="open-certificate" ${!ready || busy ? "disabled" : ""}>${icon("shield")}${state.setup.platform === "macos" ? "Импортировать сертификат" : "Открыть сертификат"}</button><button class="text-button setup-link" data-action="reveal-certificate" ${!ready || busy ? "disabled" : ""}>Показать файл</button></details>`;
  const steps = [
    `<section class="setup-step"><span class="step-number">1</span><div><h3>Подготовка</h3><p>${!helper ? installing ? progress : busy ? "Ищем mitmproxy и готовим загрузку…" : setupError ? "Подготовка остановлена." : "Ищем mitmproxy…" : !ready ? busy ? "Создаём локальный сертификат…" : "Сертификат ещё не создан." : "Перехватчик и сертификат готовы."}</p><p class="field-help">Приложение само найдёт mitmdump или загрузит проверенную версию и создаст отдельный локальный сертификат.</p>${!busy && (!helper || !ready) ? `<button class="secondary" data-action="prepare-setup">Повторить подготовку</button>` : ""}${!state.setup.installSupported && !helper ? '<p class="field-help">Для этой архитектуры укажи установленный mitmdump вручную.</p>' : ""}${showManualHelper ? `<details><summary>Указать свой mitmdump</summary><form id="helper-form"><input name="helperPath" aria-label="Путь к mitmdump" placeholder="Путь к mitmdump"/><button class="secondary setup-link" type="submit">Использовать</button></form></details>` : ""}${helper ? `<details class="helper-location"><summary>Где установлен</summary><p class="helper-path">${esc(state.setup.helperPath)}</p></details>` : ""}</div></section>`,
    `<section class="setup-step"><span class="step-number ${trusted ? "done" : ""}">${trusted ? icon("check") : "2"}</span><div><h3>Доверие сертификату <span class="pill ${trusted ? "good" : ""}">${trusted ? "Готово" : "Нужна установка"}</span></h3>
    ${state.setup.platform === "macos" ? `<p>${trusted ? "Доверие настроено. После включения перехватчика полностью выйди из Claude через ⌘Q и открой его заново." : "Добавим сертификат в связку текущего пользователя и настроим доверие для HTTPS. macOS может запросить подтверждение; пароль вводится только в системном окне."}</p><button class="primary" data-action="install-certificate" ${!ready || busy || trusted ? "disabled" : ""}>${icon("shield")}${busy && pendingAction === "install-certificate" ? "Ожидаем разрешения…" : trusted ? "Доверие настроено" : "Установить сертификат"}</button>` : ""}
    ${state.setup.platform === "windows" ? `<p>Установим текущий сертификат в доверенные корневые центры текущего пользователя Windows.</p><button class="primary" data-action="install-certificate" ${!ready || busy || trusted ? "disabled" : ""}>${icon("shield")}${trusted ? "Доверие настроено" : "Установить сертификат"}</button>` : ""}
    ${identity}
    ${systemTrustWarning()}
    ${manualTrust}
    <p class="subtle">Получение нового ответа Claude подтвердит, что клиент доверяет сертификату.</p></div></section>`,
    `<section class="setup-step"><span class="step-number ${configured ? "done" : ""}">${configured ? icon("check") : "3"}</span><div><h3>Сервер и процессы <span class="pill ${configured ? "good" : ""}">${configured ? "Выбраны" : "Не выбраны"}</span></h3><form id="setup-form">${connectionFields({ ...state.settings, ...(setupDraft || {}) }, !setupDraft)}</form></div></section>`
  ];
  return `<div class="wizard-heading"><span class="wizard-caption">Подключение · шаг ${setupStep + 1} из 3</span><div class="wizard-progress" aria-label="Шаг ${setupStep + 1} из 3">${steps.map((_, i) => `<i class="${i <= setupStep ? "active" : ""}"></i>`).join("")}</div></div>${steps[setupStep]}`;

}

function connectionFields(s: Settings, suggestDefault = false) {
  return `<fieldset><legend><label for="server">Сервер</label></legend><input type="text" id="server" name="hosts" value="${esc(s.hosts[0] || (suggestDefault ? "api.anthropic.com" : ""))}" spellcheck="false" placeholder="api.anthropic.com"/><p class="field-help">Домен или URL. Путь запросов может быть любым.</p></fieldset>
    <fieldset><legend>Процессы</legend><p class="process-defaults">По умолчанию перехватываем <code>Claude</code> и <code>claude</code>.</p><label class="field-label" for="additional-processes">Дополнительные процессы</label><input type="text" id="additional-processes" name="additionalProcesses" value="${esc(s.additionalProcesses.join(", "))}" placeholder="Code, extensionHost" spellcheck="false"/><p class="field-help">Имена процессов через запятую, с учётом регистра. Например, для конкретного расширения IDE.</p></fieldset>`;
}

let settingsTab = "general";

function settings() {
  const s = state.settings;
  return `<form id="settings-form"><div class="section-heading"><h2>Настройки</h2></div>
    <div class="settings-tabs" role="tablist" aria-label="Разделы настроек">${[['general','Общие','system'],['capture','Захват','shield'],['updates','Обновления','clock']].map(([id,label,glyph]) => `<button type="button" id="tab-${id}" role="tab" data-settings-tab="${id}" aria-controls="settings-${id}" aria-selected="${settingsTab === id}" tabindex="${settingsTab === id ? 0 : -1}">${icon(glyph)}<span>${label}</span></button>`).join("")}</div>
    <div id="settings-general" role="tabpanel" aria-labelledby="tab-general" ${settingsTab !== "general" ? "hidden" : ""}>
    <fieldset><legend>Оформление</legend><div class="theme-switch" role="group" aria-label="Тема">${([['system','Система'],['light','Светлая'],['dark','Тёмная']] as const).map(([value, label]) => `<button type="button" data-theme="${value}" class="${s.theme === value ? "selected" : ""}" aria-label="${label}" title="${label}" aria-pressed="${s.theme === value}">${icon(value)}</button>`).join("")}</div></fieldset>
    <fieldset><legend>Запуск</legend><label class="client-option"><input type="checkbox" id="launch-at-login" ${state.setup.launchAtLogin ? "checked" : ""} ${state.setup.launchAtLogin == null ? "disabled" : ""}/><span><strong>Запускать вместе с ОС</strong><small>В фоне, со значком в трее.</small></span></label><p class="field-help">${state.setup.launchAtLogin == null ? "Не удалось прочитать настройку автозапуска." : state.setup.platform === "macos" ? "Для автозапуска держи приложение в «Программах»." : "Настройка применяется сразу."}</p></fieldset>
    <fieldset><legend>Диагностика</legend><p class="field-help">Отчёт содержит только состояние компонентов. Пути, серверы и данные запросов в него не входят. Просмотри текст перед копированием.</p><button type="button" class="secondary" data-action="make-report">Собрать отчёт</button><div id="support-report">${supportReport ? `<pre class="support-report">${esc(supportReport)}</pre><button type="button" class="secondary" data-action="copy-report">Скопировать отчёт</button>` : ""}</div></fieldset>
    <section id="gnome-section"></section>
    </div><div id="settings-updates" role="tabpanel" aria-labelledby="tab-updates" ${settingsTab !== "updates" ? "hidden" : ""}><section id="updates-section"></section></div>
    <div id="settings-capture" role="tabpanel" aria-labelledby="tab-capture" ${settingsTab !== "capture" ? "hidden" : ""}>
    <fieldset><legend>Сертификат и доверие</legend><p class="field-help">Имя сертификата: <strong>${esc(state.setup.certificateName || "не удалось прочитать")}</strong></p>${certificateTrustDetails()}<details><summary>Как настроить доверие</summary>${state.setup.platform === "linux" ? `<div class="trust-guide">${linuxCertificateHelp(false)}</div>` : `<p class="field-help">${state.setup.platform === "macos" ? "В связке «Вход» текущего пользователя открой сертификат → «Доверие» → «Всегда доверять»." : "Импортируй сертификат в «Доверенные корневые центры сертификации» текущего пользователя."}</p><button type="button" class="secondary" data-action="${state.setup.platform === "windows" ? "install-certificate" : "open-certificate"}">${state.setup.platform === "windows" ? "Установить текущий сертификат" : "Открыть сертификат"}</button>`}<button type="button" class="text-button setup-link" data-action="reveal-certificate">Показать файл</button></details></fieldset>
    ${connectionFields({ ...s, ...(settingsDraft || {}) })}
    <p class="field-help">Если изменить сервер или процессы, захват перезапустится при сохранении.</p></div></form>`;
}

function renderGnome() {
  if (deferDuringPointer(renderGnome)) return;
  const section = document.querySelector<HTMLElement>("#gnome-section");
  if (!section) return;
  setMarkupIfChanged(section, `${state.setup.platform === "linux" ? `<fieldset><legend>Панель GNOME</legend><p class="field-help">${esc(gnomeStatus?.message || "Нативные проценты и прогресс для GNOME 46 (Ubuntu 24.04).")}</p>${gnomeStatus?.supported ? `<button type="button" class="secondary" data-action="install-gnome" ${busy ? "disabled" : ""}>${gnomeStatus.installed ? "Обновить расширение" : "Установить расширение"}</button>` : ""}${gnomeStatus?.pending ? `<button type="button" class="secondary" data-action="gnome-logout">Выйти из сеанса…</button><p class="field-help">Ubuntu попросит подтвердить выход. После входа расширение включится автоматически.</p>` : ""}</fieldset>` : ""}`);
}

function renderUpdates() {
  if (deferDuringPointer(renderUpdates)) return;
  const section=document.querySelector("#updates-section"); if (!section) return;
  const phase=updateState.phase;
  const text=phase === "checking" ? "Проверяем…" : phase === "downloading" ? `Загружаем${updateState.total ? ` ${Math.floor(updateState.downloaded / updateState.total * 100)}%` : "…"}` : phase === "installing" ? "Устанавливаем…" : phase === "available" ? `Доступна версия ${updateState.version}` : phase === "current" ? "Установлена актуальная версия" : phase === "unpublished" ? "Обновления пока не опубликованы" : phase === "error" ? updateState.error : "Новые версии из GitHub Releases";
  setMarkupIfChanged(section, `<fieldset><legend>Обновления</legend><p class="field-help">${esc(text)}</p></fieldset>`);
}
function renderFooter() {
  if (deferDuringPointer(renderFooter)) return;
  renderUpdates();
  renderGnome();
  const phase = state.capture.phase;
  const text = phase === "stopped" ? "Перехват выключен" : phase === "starting" ? "Запускаем захват…" : phase === "stopping" ? "Останавливаем…" : state.setup.certificateExists && state.setup.certificateTrusted === false ? "Нужно доверие к CA" : state.connectionIssue ? "Ошибка соединения" : confirmed() ? "Лимиты получены" : "Ждём ответ Claude";
  setMarkupIfChanged(document.querySelector("#header-status")!, `<span class="capture-status"><i class="status-dot ${confirmed() ? "green" : phase !== "stopped" ? "amber" : ""}"></i>${text}</span>${needsSetup(state) && phase === "stopped" ? '<button class="capture-button start" data-view="setup">Подключить</button>' : `<button class="capture-button ${phase === "stopped" ? "start" : ""}" data-action="${phase === "stopped" ? "start" : "stop"}" ${busy || phase === "stopping" ? "disabled" : ""}>${phase === "stopped" ? "Запустить" : "Остановить"}</button>`}`);
  const wizardNav = view === "setup" && setupStep === 2 ? `<div class="wizard-nav"><button class="secondary" data-action="setup-back" ${busy ? "disabled" : ""}>Назад</button><button class="primary" type="submit" form="setup-form" ${busy ? "disabled" : ""}>${busy ? "Подключаем…" : "Подключить"}</button></div>` : "";
  setMarkupIfChanged(document.querySelector("#screen-actions")!, `${wizardNav}${view === "settings" ? `<button class="primary screen-save" type="submit" form="settings-form" ${busy ? "disabled" : ""}>${busy ? "Сохраняем…" : "Сохранить"}</button>` : ""}`);
  const updatePhase = updateState.phase;
  const updateWorking = ["checking", "downloading", "installing"].includes(updatePhase);
  const updateAvailable = updatePhase === "available";
  const updateAction = updateAvailable ? updateState.manualInstall ? "open-releases" : "install-update" : "check-update";
  const updateLabel = updatePhase === "checking" ? "Проверяем…" : updatePhase === "downloading" ? `Загружаем${updateState.total ? ` ${Math.floor(updateState.downloaded / updateState.total * 100)}%` : "…"}` : updatePhase === "installing" ? "Устанавливаем…" : updateAvailable ? "Доступно обновление" : "Проверить обновления";
  const updateTitle = updateAvailable ? `${updateState.manualInstall ? "Скачать" : "Установить"} версию ${updateState.version || ""}` : updateLabel;
  setMarkupIfChanged(document.querySelector("#footer")!, `<div class="footer-bottom"><div class="footer-left"><span class="version">${esc(appVersion)}</span><button class="text-button footer-update ${updateAvailable ? "available" : ""}" data-action="${updateAction}" title="${esc(updateTitle)}" aria-label="${esc(updateTitle)}" ${updateWorking ? "disabled" : ""}>${esc(updateLabel)}</button></div><button class="text-button" data-action="quit">Выйти</button></div>`);
  const message = toast || state.setup.certificateSecurityIssue || state.setup.certificateTrustError || (!state.setup.helperPath && state.setup.installation.error) || state.capture.error || state.connectionIssue;
  const notice = document.querySelector("#notice")!;
  const retry = phase === "stopped" && !!state.capture.error && message === state.capture.error;
  setMarkupIfChanged(notice, message ? `<div class="notice">${icon("warning")}<span>${esc(message)}${retry ? '<button type="button" class="notice-retry" data-action="start">Повторить запуск</button>' : ""}</span><button class="icon-button" data-action="dismiss" aria-label="Закрыть сообщение">${icon("close")}</button></div>` : "");
}

async function refresh() {
  if (!native) return;
  try { state = await invoke<Status>("get_status"); if (firstStatus) { view = needsSetup(state) ? "setup" : "overview"; setupStep = requiredSetupStep(); firstStatus = false; } applyTheme(state.settings.theme); if (view !== "settings" || needsSetup(state)) render(); else renderFooter(); }
  catch { toast = "Не удалось получить состояние Show My Limits."; renderFooter(); }
}

async function finishSetup() {
  if (needsSetup(state)) return;
  if (native) {
    await invoke("complete_setup");
    if (state.setup.platform === "linux") {
      try {
        gnomeStatus = await invoke<GnomeStatus>("gnome_status");
        if (gnomeStatus.supported && !gnomeStatus.installed) {
          gnomeStatus = await invoke<GnomeStatus>("install_gnome_extension");
          toast = gnomeStatus.message;
        }
      } catch (error) { toast = `Перехват включён. Расширение GNOME можно установить позже в настройках: ${String(error)}`; }
    }
  }
  else { state.capture.phase = "starting"; state.capture.startedAt = state.now; }
  view = "overview";
  await refresh(); render();
}

async function prepareSetup() {
  if (busy || (state.setup.helperPath && state.setup.certificateExists)) return;
  busy = true; pendingAction = "prepare-setup"; setupError = null; toast = null; render();
  try {
    if (!native) {
      state.setup.helperPath ||= "/preview/mitmdump";
      state.setup.certificateExists = true;
    } else {
      if (!state.setup.helperPath) {
        if (!state.setup.installSupported) throw new Error("Для этой архитектуры укажи установленный mitmdump вручную.");
        await invoke("install_helper");
        await refresh();
        if (!state.setup.helperPath) throw new Error("Установленный mitmdump не найден. Повтори подготовку или укажи путь вручную.");
      }
      if (!state.setup.certificateExists) {
        await invoke("prepare_certificate");
        await refresh();
        if (!state.setup.certificateExists) throw new Error("Сертификат не создан. Повтори подготовку.");
      }
    }
    setupStep = requiredSetupStep();
  } catch (error) { setupError = String(error); toast = setupError; }
  finally { busy = false; pendingAction = ""; render(); }
}

async function action(name: string) {
  if (name === "edit-server") { settingsTab = "capture"; view = "settings"; toast = null; render(); return; }
  if (name === "make-report") {
    try {
      supportReport = native ? await invoke<string>("diagnostic_report") : "Show My Limits (browser preview)\nStatus: no native diagnostics";
      const target = document.querySelector("#support-report");
      if (target) target.innerHTML = `<pre class="support-report">${esc(supportReport)}</pre><button type="button" class="secondary" data-action="copy-report">Скопировать отчёт</button>`;
      schedulePanelSize();
    } catch (error) { toast = `Не удалось собрать диагностику: ${String(error)}`; renderFooter(); }
    return;
  }
  if (name === "copy-report" && supportReport) {
    try {
      await navigator.clipboard.writeText(supportReport);
      const button = document.querySelector('[data-action="copy-report"]');
      if (button) button.textContent = "Скопировано";
    } catch { toast = "Не удалось скопировать отчёт. Выдели текст и скопируй вручную."; renderFooter(); }
    return;
  }
  if (name === "copy-trust-commands") {
    try {
      await navigator.clipboard.writeText(linuxTrustCommands(state.setup.certificatePath));
      const button = document.querySelector('[data-action="copy-trust-commands"]');
      if (button) button.textContent = "Скопировано";
    } catch {
      toast = "Не удалось скопировать. Выдели команды и скопируй их вручную.";
      renderFooter();
    }
    return;
  }
  if (["check-update", "install-update", "open-releases"].includes(name)) {
    if (!native) { updateState.phase=name === "check-update" ? "current" : "idle"; renderFooter(); return; }
    toast = null;
    try {
      if (name === "check-update") updateState=await invoke<UpdateStatus>("check_update");
      if (name === "install-update") await invoke("install_update");
      if (name === "open-releases") await invoke("open_releases");
    } catch (error) { updateState.phase="error"; updateState.error=String(error); toast=String(error); }
    renderFooter(); return;
  }
  if (name === "dismiss") { toast = null; state.capture.error = null; state.connectionIssue = null; renderFooter(); return; }
  if (name === "prepare-setup") { await prepareSetup(); return; }
  if (name === "setup-back" && !busy) { setupStep = 1; render(); return; }
  if (!native) {
    if (["install-certificate", "confirm-trust"].includes(name)) { state.settings.certificateTrustConfirmed = true; state.setup.certificateTrusted = true; setupStep = 2; }
    if (name === "start") { state.capture.phase = "starting"; state.capture.startedAt = state.now; state.capture.error = null; }
    if (name === "stop") state.capture.phase = "stopped";
    render(); return;
  }
  if (name === "quit") { await invoke("quit_app"); return; }
  if (busy) return;
  busy = true; pendingAction = name; toast = null; if (view === "setup") render(); else renderFooter();
  try {
    if (name === "gnome-logout") await invoke("logout_gnome_session");
    if (name === "install-gnome") {
      gnomeStatus = await invoke<GnomeStatus>("install_gnome_extension");
      toast = gnomeStatus.message;
    }
    if (name === "finish-setup") await finishSetup();
    if (name === "confirm-trust") {
      const next = { ...state.settings, certificateTrustConfirmed: true };
      await invoke("save_settings", { settings: next }); state.settings = next;
    }
    if (name === "start") { await invoke("start_capture"); view = "overview"; }
    if (name === "stop") await invoke("stop_capture");
    if (name === "install-certificate") await invoke("install_certificate");
    if (name === "open-certificate") await invoke("open_certificate");
    if (name === "reveal-certificate") await invoke("reveal_certificate");
    await refresh();
    if (view === "setup" && ["install-certificate", "confirm-trust"].includes(name)
      && certificateTrusted(state)) setupStep = 2;
    if (view === "setup") render();
  } catch (error) { toast = String(error); }
  finally { busy = false; pendingAction = ""; if (view === "setup") render(); else renderFooter(); }
}

root.addEventListener("mousedown", event => {
  if (!native || state.setup.platform !== "linux" || event.button !== 0) return;
  const target = event.target as HTMLElement;
  if (!target.closest(".brand") || target.closest("button,a,input,select,textarea")) return;
  event.preventDefault();
  void invoke("start_window_drag").catch(error => {
    toast = `Не удалось переместить окно: ${String(error)}`;
    renderFooter();
  }).finally(finishPointerInteraction);
});

root.addEventListener("click", async event => {
  const target = (event.target as HTMLElement).closest<HTMLElement>("[data-view],[data-action],[data-theme]");
  if (!target) return;
  if (target.dataset.view) {
    view = needsSetup(state) ? "setup" : target.dataset.view === "settings" && view !== "settings" ? "settings" : "overview";
    if (native && state.setup.platform === "linux" && view === "settings") {
      try { gnomeStatus = await invoke<GnomeStatus>("gnome_status"); } catch { /* keep fallback */ }
    }
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

function selectSettingsTab(id: string, focus = false) {
  settingsTab = id;
  root.querySelectorAll<HTMLElement>("[data-settings-tab]").forEach(button => {
    const selected = button.dataset.settingsTab === id;
    button.setAttribute("aria-selected", String(selected));
    button.tabIndex = selected ? 0 : -1;
    if (selected && focus) button.focus();
  });
  root.querySelectorAll<HTMLElement>('[role="tabpanel"]').forEach(panel => { panel.hidden = panel.id !== `settings-${id}`; });
}
root.addEventListener("click", event => {
  const tab = (event.target as HTMLElement).closest<HTMLElement>("[data-settings-tab]");
  if (tab) selectSettingsTab(tab.dataset.settingsTab!);
});
root.addEventListener("keydown", event => {
  const tab = (event.target as HTMLElement).closest<HTMLElement>("[data-settings-tab]");
  if (!tab || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const ids = ["general", "capture", "updates"];
  const index = ids.indexOf(tab.dataset.settingsTab!);
  selectSettingsTab(ids[event.key === "Home" ? 0 : event.key === "End" ? 2 : (index + (event.key === "ArrowRight" ? 1 : 2)) % 3], true);
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
  let helperSaved = false;
  busy = true; toast = null; renderFooter();
  try {
    if (form.id !== "helper-form" && !next.hosts.length) throw new Error("Укажи хотя бы один сервер.");
    if (native) await invoke("save_settings", { settings: next });
    state.settings = next;
    if (!native && form.id === "helper-form") state.setup.helperPath = next.helperPath || null;
    if (native) await refresh();
    if (form.id === "helper-form") helperSaved = true;
    if (needsSetup(state)) view = "setup";
    else if (form.id === "setup-form") { setupDraft = null; await finishSetup(); }
    else if (form.id === "settings-form") { settingsDraft = null; view = "overview"; }
  } catch (error) { toast = String(error); }
  finally { busy = false; render(); if (helperSaved) void prepareSetup(); }
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
  ["quickstart", "Быстрый старт"], ["trust-lost", "Доверие к CA удалено"], ["system-ca", "CA в системе"], ["stopped", "Перехват выключен"], ["capture-error", "Ошибка запуска захвата"], ["settings", "Настройки"], ["update-available", "Доступно обновление"],
  ["setup-0", "1 · Подготовка"], ["setup-1", "2 · Доверие"], ["setup-2", "3 · Сервер и процессы"],
];
function setDevScene(scene: string) {
  if (!devTools) return;
  if (!devScenes.some(([id]) => id === scene)) scene = "limits";
  const theme = state.settings.theme;
  state = demoStatus(); state.settings.theme = theme;
  updateState = scene === "update-available"
    ? { phase: "available", version: "0.2.2", downloaded: 0, total: null, error: null, manualInstall: false }
    : { phase: "idle", version: null, downloaded: 0, total: null, error: null, manualInstall: false };
  const platform = new URLSearchParams(location.search).get("platform");
  if (["linux", "windows", "macos"].includes(platform || "")) state.setup.platform = platform!;
  if (state.setup.platform === "linux") state.setup.certificatePath = "/home/demo/.local/share/io.github.rowro.showmylimits/certificates/mitmproxy-ca-cert.cer";
  setupDraft = null; settingsDraft = null; toast = null; setupError = null; busy = false; pendingAction = "";
  view = scene === "settings" ? "settings" : "overview";
  if (scene === "trust-lost") state.setup.certificateTrusted = false;
  if (scene === "system-ca") {
    state.setup.certificateTrustScope = "system";
    state.setup.certificateSha256 = "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
  }
  if (scene === "low" || scene === "exhausted") {
    state.profiles[0].windows["5h"].utilization = scene === "low" ? 0.92 : 1;
    state.profiles[0].windows["7d"].utilization = 0.84;
    if (scene === "exhausted") state.profiles[0].status = "rejected";
  }
  if (scene === "stale") state.profiles[0].observedAt = state.now - 7200;
  if (scene === "quickstart" || scene === "stopped" || scene === "capture-error") {
    state.profiles = [];
    state.capture.startedAt = scene === "quickstart" ? state.now : null;
    state.capture.phase = scene === "quickstart" ? "listening" : "stopped";
    if (scene === "capture-error") state.capture.error = "Не удалось открыть графический запрос прав для перехвата Linux. Проверьте установку Show My Limits и наличие zenity.";
  }
  if (scene.startsWith("setup-")) {
    setupStep = Number(scene.slice(-1)); view = "setup";
    state.profiles = []; state.settings.hosts = [];
    state.capture.phase = "stopped"; state.capture.startedAt = null;
    state.setup.helperPath = setupStep > 0 ? "/example/mitmdump" : null;
    state.setup.certificateExists = setupStep > 0;
    state.settings.certificateTrustConfirmed = setupStep > 1; state.setup.certificateTrusted = setupStep > 1;
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
  const scene = params.get("screen") || (existingDemo ? ({ restart: "quickstart", wizard: "setup-0", setup: "setup-2" }[existingDemo] || "limits") : sessionStorage.getItem("sml-dev-scene") || "limits");
  setDevScene(scene);
}

shell();
mountDevTools();
const panelObserver = new ResizeObserver(schedulePanelSize);
["#content-inner", "header", "#screen-actions", "#notice", "#footer"].forEach(selector => panelObserver.observe(document.querySelector(selector)!));
root.addEventListener("toggle", schedulePanelSize, true);
// Do not replace a pressed button or move it while input blur changes layout.
root.addEventListener("pointerdown", () => { pointerActive = true; }, true);
function finishPointerInteraction() {
  if (!pointerActive) return;
  pointerActive = false;
  requestAnimationFrame(() => {
    const pending = [...deferredRenders];
    deferredRenders.clear();
    pending.forEach(render => render());
    schedulePanelSize();
  });
}
window.addEventListener("pointerup", finishPointerInteraction);
window.addEventListener("pointercancel", finishPointerInteraction);
window.addEventListener("blur", finishPointerInteraction);
root.addEventListener("focusout", schedulePanelSize);
window.addEventListener("focus", () => { lastDesiredHeight = 0; schedulePanelSize(); });
window.addEventListener("resize", () => { if (!native) { lastDesiredHeight = 0; schedulePanelSize(); } });
void document.fonts.ready.then(schedulePanelSize);
if (native) {
  appVersion=await getVersion();
  updateState=await invoke<UpdateStatus>("update_status");
  await listen<UpdateStatus>("show-my-limits://update", event => { updateState=event.payload; renderFooter(); });
  await refresh();
  if (needsSetup(state) && requiredSetupStep() === 0) void prepareSetup();
  if (state.setup.platform === "linux") {
    try { gnomeStatus = await invoke<GnomeStatus>("gnome_status"); } catch { /* ordinary tray remains available */ }
  }
  await listen<string>("show-my-limits://navigate", async event => {
    view = needsSetup(state) ? "setup" : event.payload === "settings" ? "settings" : "overview";
    if (state.setup.platform === "linux") {
      try { gnomeStatus = await invoke<GnomeStatus>("gnome_status"); } catch { /* ordinary tray remains available */ }
    }
    toast = null;
    render();
  });
  await listen<Status>("show-my-limits://status", event => {
    state = event.payload; applyTheme(state.settings.theme);
    if (view !== "settings" || needsSetup(state)) render(); else renderFooter();
  });
} else {
  setInterval(() => { state.now = Date.now() / 1000; if (view === "overview") render(); }, 1000);
}
