import { cp, mkdir, readFile, stat } from 'node:fs/promises';
import { basename, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// Explicit local packaging step. No download or global installation is performed.
const source = process.argv[2] || process.env.SML_MITMDUMP;
if (!source) {
  console.error('Usage: npm run prepare:helper -- /path/to/standalone/mitmdump[.exe]\nA full standalone distribution directory is also accepted.');
  process.exit(1);
}
const root = fileURLToPath(new URL('../', import.meta.url));
const input = resolve(source);
const destination = resolve(root, 'src-tauri/binaries');
await mkdir(destination, { recursive:true });
if ((await stat(input)).isDirectory()) {
  await cp(input, resolve(destination, 'mitmproxy'), { recursive:true });
  console.log('Copied standalone distribution. Its mitmdump executable must be at binaries/mitmproxy/mitmdump[.exe].');
} else {
  const bytes=await readFile(input);
  if (bytes.subarray(0,2).toString()==='#!') throw new Error('This is a Python entrypoint, not a self-contained binary. Use an official standalone distribution.');
  const executable=process.platform==='win32'?'mitmdump.exe':'mitmdump';
  await cp(input,resolve(destination,executable),{preserveTimestamps:true});
  console.log(`Copied ${basename(input)} to binaries/${executable}. Verify this standalone binary and required capture components on a clean target OS before releasing.`);
}
