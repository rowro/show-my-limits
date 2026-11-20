import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';
import Pango from 'gi://Pango';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {derive, percent, remaining, resetText} from './model.js';

const BUS = 'io.github.rowro.ShowMyLimits';
const PATH = '/io/github/rowro/ShowMyLimits';
const IFACE = 'io.github.rowro.ShowMyLimits1';
function label(text, style = '') {
    const actor = new St.Label({text, style_class: style, x_expand: true});
    actor.clutter_text.line_wrap = true;
    actor.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
    return actor;
}
function progress(style) {
    const actor = new St.DrawingArea({style_class: style, x_expand: true, y_align: Clutter.ActorAlign.CENTER});
    actor.fraction = null;
    actor.connect('repaint', () => {
        const cr = actor.get_context();
        const [width, height] = actor.get_surface_size();
        const color = actor.get_theme_node().get_foreground_color();
        cr.setSourceRGBA(color.red / 255, color.green / 255, color.blue / 255, 0.2);
        cr.rectangle(0, 0, width, height);
        cr.fill();
        const value = actor.fraction;
        if (value != null) {
            const rgb = value <= 0.1 ? [0.84, 0.35, 0.31] : value <= 0.3 ? [0.85, 0.62, 0.24] : [0.31, 0.66, 0.51];
            cr.setSourceRGBA(...rgb, 1);
            cr.rectangle(0, 0, width * value, height);
            cr.fill();
        }
        cr.$dispose();
    });
    return actor;
}
function quotaCard(name) {
    const box = new St.BoxLayout({vertical: true, style_class: 'sml-quota'});
    const row = new St.BoxLayout({style_class: 'sml-quota-row'});
    const title = label(name, 'sml-heading');
    const value = new St.Label({text: '—', style_class: 'sml-quota-percent'});
    row.add_child(title);
    row.add_child(value);
    box.add_child(row);
    const bar = progress('sml-quota-progress');
    box.add_child(bar);
    const used = label('Использовано —', 'sml-muted');
    const reset = label('Время сброса неизвестно', 'sml-muted');
    box.add_child(used);
    box.add_child(reset);
    return {box, value, bar, used, reset};
}
const LimitsIndicator = GObject.registerClass(class LimitsIndicator extends PanelMenu.Button {
    _init(openApp, quitApp) {
        super._init(0.0, 'Show My Limits');
        this._openApp = openApp;
        this._quitApp = quitApp;
        const box = new St.BoxLayout({style_class: 'sml-panel-box'});
        this._percentage = new St.Label({text: '—', style_class: 'sml-panel-label', y_align: Clutter.ActorAlign.CENTER});
        this._bar = progress('sml-panel-progress');
        this._bar.x_expand = false;
        box.add_child(this._percentage);
        box.add_child(this._bar);
        this.add_child(box);

        const content = new PopupMenu.PopupBaseMenuItem({reactive: false, can_focus: false});
        const column = new St.BoxLayout({vertical: true, style_class: 'sml-limits'});
        column.add_child(label('Show My Limits', 'sml-heading'));
        this._host = label('', 'sml-muted');
        this._status = label('Приложение не запущено', 'sml-muted');
        column.add_child(this._host);
        column.add_child(this._status);
        this._cards = {'5h': quotaCard('5 часов'), '7d': quotaCard('7 дней')};
        for (const card of Object.values(this._cards))
            column.add_child(card.box);
        content.add_child(column);
        this.menu.addMenuItem(content);
        this._openItem = this.menu.addAction('Открыть приложение', () => this._openApp('overview'));

        this._context = new PopupMenu.PopupMenu(this, 0.0, St.Side.TOP);
        Main.uiGroup.add_child(this._context.actor);
        this._context.actor.hide();
        Main.panel.menuManager.addMenu(this._context);
        this._context.addAction('Открыть приложение', () => this._openApp('overview'));
        this._context.addAction('Настройки', () => this._openApp('settings'));
        this._quitItem = this._context.addAction('Выйти из Show My Limits', () => this._quitApp());
    }
    vfunc_event(event) {
        const type = event.type();
        if (type === Clutter.EventType.BUTTON_PRESS || type === Clutter.EventType.TOUCH_BEGIN) {
            const right = type === Clutter.EventType.BUTTON_PRESS && event.get_button() === Clutter.BUTTON_SECONDARY;
            if (right) {
                this.menu.close();
                this._context.toggle();
            } else if (type === Clutter.EventType.TOUCH_BEGIN || event.get_button() === Clutter.BUTTON_PRIMARY) {
                this._context.close();
                this.menu.toggle();
            }
            return Clutter.EVENT_STOP;
        }
        if (type === Clutter.EventType.KEY_PRESS && [Clutter.KEY_Return, Clutter.KEY_space].includes(event.get_key_symbol())) {
            this._context.close();
            this.menu.toggle();
            return Clutter.EVENT_STOP;
        }
        return Clutter.EVENT_PROPAGATE;
    }
    update(snapshot) {
        const now = Date.now() / 1000;
        const state = derive(snapshot, now);
        this._percentage.text = state.title;
        this.accessible_name = `Show My Limits · ${state.title} · ${state.status}`;
        this._bar.fraction = state.fraction;
        this._bar.queue_repaint();
        this._status.text = state.status;
        this._host.text = state.profile?.host || '';
        this._host.visible = Boolean(state.profile?.host);
        this._openItem.label.text = state.online ? 'Открыть приложение' : 'Запустить Show My Limits';
        this._quitItem.setSensitive(state.online);
        for (const [key, card] of Object.entries(this._cards)) {
            const window = state.profile?.windows?.[key];
            const left = remaining(window, now);
            card.box.visible = Boolean(state.profile);
            card.value.text = `${percent(left)} осталось`;
            card.bar.fraction = left;
            card.bar.queue_repaint();
            card.used.text = `Использовано ${Number.isFinite(window?.utilization) && window.utilization >= 0 ? percent(window.utilization) : '—'}`;
            card.reset.text = resetText(window?.reset, now);
        }
    }
    destroy() {
        this._context?.destroy();
        this._context = null;
        super.destroy();
    }
});

export default class ShowMyLimitsExtension extends Extension {
    enable() {
        this._enabled = true;
        this._owner = null;
        this._pending = false;
        this._cancellable = new Gio.Cancellable();
        this._indicator = new LimitsIndicator(view => this._openApp(view), () => this._command('Quit'));
        Main.panel.addToStatusArea(this.uuid, this._indicator);
        this._indicator.update(null);
        this._watcher = Gio.bus_watch_name(Gio.BusType.SESSION, BUS, Gio.BusNameWatcherFlags.NONE,
            (_connection, _name, owner) => { if (this._enabled) { this._owner = owner; this._poll(); } },
            () => { if (this._enabled) { this._owner = null; this._indicator?.update(null); } });
        this._timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 2, () => {
            this._poll();
            return GLib.SOURCE_CONTINUE;
        });
    }
    _call(method, parameters = null, signature = null) {
        const owner = this._owner;
        return new Promise((resolve, reject) => {
            Gio.DBus.session.call(owner, PATH, IFACE, method, parameters,
                signature ? new GLib.VariantType(signature) : null,
                Gio.DBusCallFlags.NO_AUTO_START, 1500, this._cancellable,
                (connection, result) => {
                    try { resolve(connection.call_finish(result).deep_unpack()); }
                    catch (error) { reject(error); }
                });
        });
    }
    async _poll() {
        if (!this._enabled || !this._owner || this._pending)
            return;
        this._pending = true;
        const owner = this._owner;
        const session = this._cancellable;
        try {
            const [json] = await this._call('GetState', null, '(s)');
            if (!this._enabled || this._owner !== owner || this._cancellable !== session)
                return;
            if (json.length > 65536)
                throw new Error('Invalid state size');
            const snapshot = JSON.parse(json);
            if (snapshot?.protocol !== 1 || !Number.isFinite(snapshot.now) || Math.abs(Date.now() / 1000 - snapshot.now) > 10)
                throw new Error('Unsupported or stale state');
            // A rendering failure must never keep the fallback tray hidden.
            this._indicator.update(snapshot);
            const [accepted] = await this._call('Heartbeat', new GLib.Variant('(u)', [1]), '(b)');
            if (!accepted)
                throw new Error('Protocol mismatch');
        } catch {
            if (this._enabled && this._owner === owner && this._cancellable === session)
                this._indicator.update(null);
        } finally {
            if (this._cancellable === session) this._pending = false;
        }
    }
    async _command(method, parameters = null) {
        if (!this._owner)
            return;
        try { await this._call(method, parameters); }
        catch { if (this._enabled) Main.notify('Show My Limits', 'Приложение не отвечает. Открой его из меню приложений.'); }
    }
    _openApp(view) {
        if (this._owner) {
            this._command('OpenWindow', new GLib.Variant('(s)', [view]));
            return;
        }
        const app = ['Show My Limits.desktop', 'show-my-limits.desktop', 'io.github.rowro.showmylimits.desktop']
            .map(name => Gio.DesktopAppInfo.new(name)).find(Boolean);
        if (app)
            app.launch([], global.create_app_launch_context(0, -1));
        else
            Main.notify('Show My Limits', 'Открой установленное приложение из меню Ubuntu. Для AppImage запусти его файл.');
    }
    disable() {
        this._enabled = false;
        if (this._owner) {
            // Best effort detach is independent of the cancelled polling requests.
            Gio.DBus.session.call(this._owner, PATH, IFACE, 'Detach', null, null,
                Gio.DBusCallFlags.NO_AUTO_START, 1000, null, null);
        }
        this._cancellable?.cancel();
        this._cancellable = null;
        if (this._watcher) Gio.bus_unwatch_name(this._watcher);
        if (this._timer) GLib.Source.remove(this._timer);
        this._watcher = this._timer = 0;
        this._owner = null;
        this._indicator?.destroy();
        this._indicator = null;
    }
}
