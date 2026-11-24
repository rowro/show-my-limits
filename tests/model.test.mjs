import assert from 'node:assert/strict';
import test from 'node:test';
import { remaining, isFresh, countdown, percentage, needsSetup, demoStatus, needsClientRestart } from '../.test-build/model.js';

test('remaining quota is not used quota', () => {
  assert.equal(remaining({utilization:.75,reset:2000},1000),.25);
  assert.equal(percentage(.25),'25%');
  assert.equal(remaining({utilization:1.2,reset:2000},1000),0);
});
test('expired, missing and invalid values remain unknown', () => {
  assert.equal(remaining({utilization:.5,reset:900},1000),null);
  for (const value of [NaN,Infinity,-1]) assert.equal(remaining({utilization:value},1000),null);
  assert.equal(remaining(undefined,1000),null);
});
test('freshness and countdown do not invent a reset', () => {
  const profile={observedAt:1000};
  assert.equal(isFresh(profile,1599),true);
  assert.equal(isFresh(profile,1600),false);
  assert.equal(countdown(900,1000),'Ждём новый ответ Claude');
  assert.equal(countdown(1001,1000),'через 1 мин');
});

test('setup requires helper, certificate, trust, server; default processes need no selection', () => {
  const ready=demoStatus(); ready.profiles=[];
  assert.equal(needsSetup(ready),false);
  for (const mutate of [s=>s.setup.helperPath=null,s=>s.setup.certificateExists=false,s=>s.setup.certificateTrusted=false,s=>s.settings.hosts=[]]) {
    const missing=structuredClone(ready); mutate(missing); assert.equal(needsSetup(missing),true);
  }
  const existing=demoStatus(); existing.settings.certificateTrustConfirmed=false;
  assert.equal(needsSetup(existing),false);
});

test('current OS trust overrides saved confirmation and historical quota', () => {
  const state=demoStatus(); state.setup.certificateTrusted=false;
  assert.equal(needsSetup(state),true);
  state.setup.certificateTrusted=true; state.settings.certificateTrustConfirmed=false;
  assert.equal(needsSetup(state),false);
});

test('restart reminder waits for a quota response from this capture session', () => {
  const state=demoStatus();
  state.capture.startedAt=state.now; state.profiles[0].observedAt=state.now-1;
  assert.equal(needsClientRestart(state),true);
  state.capture.phase='starting';
  assert.equal(needsClientRestart(state),false);
  state.capture.phase='listening';
  state.profiles[0].observedAt=state.now+1;
  assert.equal(needsClientRestart(state),false);
  state.now+=3600;
  assert.equal(needsClientRestart(state),false);
  state.profiles=[]; state.capture.phase='stopped';
  assert.equal(needsClientRestart(state),false);
});
