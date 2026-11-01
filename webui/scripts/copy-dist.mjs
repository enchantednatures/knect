#!/usr/bin/env node
// Copies the Vite build into the rust-embed source folder. The daemon embeds
// this directory at compile time, so the binary needs no Node runtime.
import { cp, mkdir, readdir, rm, stat } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const webuiRoot = resolve(here, '..');
const source = join(webuiRoot, 'dist');
const target = resolve(webuiRoot, '..', 'crates', 'knectd', 'assets', 'webui');

async function exists(path) {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

if (!(await exists(join(source, 'index.html')))) {
  console.error(`missing ${source}/index.html — run "npm run build" first`);
  process.exit(1);
}

await rm(target, { recursive: true, force: true });
await mkdir(target, { recursive: true });
await cp(source, target, { recursive: true });

const entries = await readdir(target, { recursive: true });
console.log(`embedded ${entries.length} WebUI files into ${target}`);
