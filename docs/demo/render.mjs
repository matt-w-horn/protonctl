// Renders storyboard.html to protonctl-demo.mp4 and protonctl-demo.gif.
// The GIF is committed; the MP4 is not, because Anthropic's plugin directory
// stops validating at any file of 5 MiB or more (RFC-0001 Q36), so
// publish the MP4 elsewhere and link it from the README.
//
// Needs Node, ffmpeg with libx264, and these packages in DEMO_DEPS, a folder
// outside the repository that holds node_modules:
//   (cd /path/to/deps && npm install playwright @fontsource/inter @fontsource/jetbrains-mono)
// Run from the repository root:
//   DEMO_DEPS=/path/to/deps node docs/demo/render.mjs
import { createRequire } from 'module';
import { fileURLToPath } from 'url';
import { spawn, spawnSync } from 'child_process';
import fs from 'fs';
import os from 'os';
import path from 'path';

const here = path.dirname(fileURLToPath(import.meta.url));
if (!process.env.DEMO_DEPS) throw new Error('set DEMO_DEPS to a folder outside the repository that holds node_modules');
const require = createRequire(path.join(process.env.DEMO_DEPS, 'noop.js'));
const { chromium } = require('playwright');
const FPS = 30;

const font = (pkg, file, family, weight) => {
  const b64 = fs.readFileSync(require.resolve(`${pkg}/files/${file}`)).toString('base64');
  return `@font-face{font-family:"${family}";font-weight:${weight};src:url(data:font/woff2;base64,${b64}) format("woff2");}`;
};
const faces = [
  ...[400, 500, 600, 700].map(w => font('@fontsource/inter', `inter-latin-${w}-normal.woff2`, 'Inter', w)),
  ...[400, 500, 600, 700].map(w => font('@fontsource/jetbrains-mono', `jetbrains-mono-latin-${w}-normal.woff2`, 'JetBrains Mono', w)),
].join('');
const html = fs.readFileSync(path.join(here, 'storyboard.html'), 'utf8').replace('<style>', `<style>${faces}`);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
await page.setContent(html, { waitUntil: 'load' });
// Stop the storyboard's real-time preview; frames come from __render(t) alone.
await page.evaluate(() => { window.__still = true; });
await page.evaluate(() => document.fonts.ready);
const families = await page.evaluate(() => [...document.fonts].filter(f => f.status === 'loaded').map(f => f.family));
if (!families.some(f => f.includes('Inter')) || !families.some(f => f.includes('JetBrains'))) {
  throw new Error(`fonts did not load: ${families.join(', ')}`);
}
const duration = await page.evaluate(() => window.__duration);
const frames = Math.round(duration * FPS);

const mp4 = path.join(here, 'protonctl-demo.mp4');
const ff = spawn('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'image2pipe', '-framerate', String(FPS), '-i', '-',
  '-c:v', 'libx264', '-preset', 'slow', '-crf', '16', '-pix_fmt', 'yuv420p', '-tune', 'animation',
  '-movflags', '+faststart', mp4], { stdio: ['pipe', 'inherit', 'inherit'] });
const done = new Promise((ok, fail) => {
  ff.on('error', fail);
  ff.on('close', c => (c === 0 ? ok() : fail(new Error(`ffmpeg exited ${c}`))));
});
for (let i = 0; i < frames; i++) {
  await page.evaluate(t => window.__render(t), i / FPS);
  const png = await page.screenshot({ type: 'png' });
  if (!ff.stdin.write(png)) await new Promise(r => ff.stdin.once('drain', r));
}
ff.stdin.end();
await done;
await browser.close();

const gif = path.join(here, 'protonctl-demo.gif');
// 8 frames per second at 720 pixels keeps the GIF under 5 MiB (4.1 MiB on
// 2026-10-05); 12 at 720 measured 5.5 MiB.
const scale = 'fps=8,scale=720:-1:flags=lanczos';
const palette = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'protonctl-demo-')), 'palette.png');
const run = args => {
  const r = spawnSync('ffmpeg', ['-y', '-loglevel', 'error', ...args], { stdio: 'inherit' });
  if (r.status !== 0) throw new Error(`ffmpeg failed: ${args.join(' ')}`);
};
run(['-i', mp4, '-vf', `${scale},palettegen=max_colors=160:stats_mode=diff`, palette]);
run(['-i', mp4, '-i', palette, '-lavfi', `${scale}[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle`, gif]);
fs.rmSync(path.dirname(palette), { recursive: true, force: true });
console.log(`${frames} frames at ${FPS} fps; wrote ${path.relative(process.cwd(), mp4)} and ${path.relative(process.cwd(), gif)}`);
