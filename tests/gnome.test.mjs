import {test} from 'node:test';
import assert from 'node:assert/strict';
import {derive, percent, remaining, resetText} from '../integrations/gnome/model.js';
const now = 10000;
const normal = {protocol: 1, now, phase: 'listening', setupComplete: true, connectionIssue: false,
    profile: {host: 'gateway.example', observedAt: now - 2,
        windows: {'5h': {utilization: 0.35, reset: now + 3600}, '7d': {utilization: 0.42, reset: now + 86400}}}};
test('GNOME displays remaining quota and distinguishes blocked from missing', () => {
    assert.equal(derive(normal, now).title, '65%');
    assert.equal(percent(remaining(normal.profile.windows['7d'], now)), '58%');
    assert.equal(percent(0.42), '42%');
    assert.equal(percent(0.654), '65,4%');
    assert.equal(remaining({utilization: 1}, now), 0);
    assert.equal(remaining({utilization: 1.2}, now), 0);
    assert.equal(remaining({utilization: 0}, now), 1);
    for (const value of [null, {}, {utilization: -1}, {utilization: NaN}, {utilization: 0.5, reset: now}])
        assert.equal(remaining(value, now), null);
});
test('GNOME never presents a stale, stopped or disconnected value as a live tray percentage', () => {
    assert.equal(derive(normal, now + 601).title, '—');
    assert.equal(derive({...normal, connectionIssue: true}, now).title, '—');
    assert.equal(derive({...normal, phase: 'stopped'}, now).title, '—');
    assert.equal(derive({...normal, profile: null}, now).title, '—');
    assert.equal(derive(null, now).online, false);
    assert.equal(derive({...normal, protocol: 99}, now).online, false);
    assert.match(derive({...normal, setupComplete: false}, now).status, /подключение/);
    assert.equal(derive({...normal, setupComplete: false}, now).title, '—');
});
test('GNOME reset text does not invent a quota refill or invalid date', () => {
    assert.match(resetText(now - 1, now), /прошло/);
    assert.match(resetText(undefined, now), /неизвестно/);
    assert.match(resetText(Infinity, now), /неизвестно/);
    assert.match(resetText(now + 3660, now), /1 ч 1 мин/);
});
