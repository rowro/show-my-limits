import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
export async function makeManifest(assets, signaturesDir, version, repository, tag) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Stable semver required');
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error('Invalid repository');
  const platforms = {};
  for (const [key, extension] of Object.entries({'darwin-aarch64':'.app.tar.gz','darwin-x86_64':'.app.tar.gz','windows-x86_64':'.exe','linux-x86_64':'.AppImage'})) {
    const matches=assets.filter(asset=>asset.name.includes(`_${key}`) && asset.name.endsWith(extension));
    if (matches.length !== 1) throw new Error(`Expected one updater asset for ${key}; got ${matches.length}`);
    const asset=matches[0];
    if (!assets.some(item=>item.name===`${asset.name}.sig`)) throw new Error(`Missing signature for ${key}`);
    const signature=(await readFile(join(signaturesDir,`${asset.name}.sig`),'utf8')).trim();
    if (!signature || signature.length < 80) throw new Error(`Invalid signature for ${key}`);
    platforms[key]={ signature, url:`https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(asset.name)}` };
  }
  return {version, notes:'Show My Limits '+version, pub_date:new Date().toISOString(), platforms};
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [assetsFile, signaturesDir, output]=process.argv.slice(2);
  const {version}=JSON.parse(await readFile(new URL('../package.json',import.meta.url),'utf8'));
  const {assets}=JSON.parse(await readFile(assetsFile,'utf8'));
  const manifest=await makeManifest(assets,signaturesDir,version,process.env.GH_REPO || 'rowro/show-my-limits',process.env.RELEASE_TAG || `v${version}`);
  await writeFile(output,JSON.stringify(manifest,null,2)+'\n');
}
