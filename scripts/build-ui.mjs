#!/usr/bin/env node
// Pin NODE_ENV=production; see vite.config
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

const MODE = 'production';
process.env.NODE_ENV = MODE;

const root = resolve(import.meta.dirname ?? '.', '..');

// Guard: WSL-side builds (scripts/build-windows-exe.sh) share this
// node_modules; their npm steps can prune the other platform's native
// binaries. Detect that state and point at the fix instead of failing with an
// opaque MODULE_NOT_FOUND from rollup/vite. (Do NOT self-heal with
// `npm i --no-save <pkg>` — on this tree that prunes unrelated packages,
// see npm/cli#4828.)
import { existsSync } from 'node:fs';
const rollupNative =
  process.platform === 'win32'
    ? '@rollup/rollup-win32-x64-msvc'
    : process.platform === 'linux' && process.arch === 'x64'
      ? '@rollup/rollup-linux-x64-gnu'
      : `@rollup/rollup-${process.platform}-${process.arch}`;
const esbuildNative =
  process.platform === 'win32'
    ? '@esbuild/win32-x64'
    : `@esbuild/${process.platform}-${process.arch}`;
const missing = [
  resolve(root, 'node_modules', ...rollupNative.split('/')),
  resolve(root, 'node_modules', ...esbuildNative.split('/')),
  resolve(root, 'node_modules', 'vite', 'bin', 'vite.js'),
].filter((p) => !existsSync(p));
if (missing.length) {
  console.error(
    `[build-ui] node_modules is incomplete (native/binary packages missing on ${process.platform}):\n  ` +
      missing.join('\n  ') +
      '\nRun `npm ci` to restore, then retry.',
  );
  process.exit(1);
}
const viteBin = resolve(root, 'node_modules', 'vite', 'bin', 'vite.js');
const r = spawnSync(process.execPath, [viteBin, 'build', '--config', 'src/ui/vite.config.ts', '--mode', MODE], {
  cwd: root,
  stdio: 'inherit',
});
process.exit(r.status ?? 1);
