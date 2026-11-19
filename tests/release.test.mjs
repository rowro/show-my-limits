import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {makeManifest} from '../scripts/update-manifest.mjs';
test('manifest requires every platform and matching signatures',async()=>{
 const directory=await mkdtemp(join(tmpdir(),'sml-manifest-'));
 try {
  const names=['show-my-limits_0.1.0_darwin-aarch64.app.tar.gz','show-my-limits_0.1.0_darwin-x86_64.app.tar.gz','show-my-limits_0.1.0_windows-x86_64-setup.exe','show-my-limits_0.1.0_linux-x86_64.AppImage'];
  const assets=names.flatMap(name=>[{name},{name:name+'.sig'}]);
  for(const name of names) await writeFile(join(directory,name+'.sig'),'s'.repeat(128));
  const manifest=await makeManifest(assets,directory,'0.1.0','rowro/show-my-limits','v0.1.0');
  assert.equal(Object.keys(manifest.platforms).length,4);
  assert.match(manifest.platforms['windows-x86_64'].url,/v0.1.0.*setup.exe$/);
  await assert.rejects(makeManifest(assets.slice(2),directory,'0.1.0','rowro/show-my-limits','v0.1.0'));
  await assert.rejects(makeManifest(assets.filter(a=>!a.name.endsWith('AppImage.sig')),directory,'0.1.0','rowro/show-my-limits','v0.1.0'));
 }finally{await rm(directory,{recursive:true,force:true});}
});
