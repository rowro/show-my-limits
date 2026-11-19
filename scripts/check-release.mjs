import { readFile } from 'node:fs/promises';
const pkg=JSON.parse(await readFile(new URL('../package.json',import.meta.url),'utf8'));
const conf=JSON.parse(await readFile(new URL('../src-tauri/tauri.conf.json',import.meta.url),'utf8'));
const cargo=await readFile(new URL('../src-tauri/Cargo.toml',import.meta.url),'utf8');
const version=cargo.match(/^version = "([^"]+)"/m)?.[1];
if (pkg.version !== conf.version || pkg.version !== version) throw new Error('Versions differ between package.json, Cargo.toml and tauri.conf.json');
if (!conf.plugins?.updater?.pubkey) throw new Error('Updater public key is missing');
if (!conf.plugins.updater.endpoints.every(url=>url.startsWith('https://github.com/rowro/show-my-limits/'))) throw new Error('Unexpected updater endpoint');
console.log('Release configuration OK:',pkg.version);
