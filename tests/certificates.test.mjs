import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {shellQuote,linuxTrustCommands} from '../.test-build/certificates.js';

test('certificate paths remain literal shell arguments', {skip:process.platform==='win32'}, () => {
  const path="/home/Test User/o'neil/$(echo unsafe);cert.cer";
  const result=spawnSync('sh',['-c',`printf '%s' ${shellQuote(path)}`],{encoding:'utf8'});
  assert.equal(result.status,0);
  assert.equal(result.stdout,path);
  const commands=linuxTrustCommands(path);
  assert.match(commands,/install -D -m 644 --/);
  assert.ok(commands.endsWith('/usr/local/share/ca-certificates/show-my-limits.crt\nsudo update-ca-certificates'));
  assert.equal(commands.split("\n").length,2);
  assert.doesNotMatch(commands,/mitmproxy-ca\.pem/);
});
