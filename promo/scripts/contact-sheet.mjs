// Pulls single stills (t = 1 s, 3 s, 5 s, ...) out of a rendered MP4 into out/frames/,
// using the ffmpeg that ships with Remotion (no filters needed: seek + one frame).
// The contact sheet itself is the Contact16x9 / Contact1x1 composition (npm run contact).
// Usage: node scripts/contact-sheet.mjs [16x9|1x1]
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);
const variant = process.argv[2] ?? "16x9";
const input = path.resolve(`out/pigeon-promo-${variant}.mp4`);
if (!existsSync(input)) throw new Error(`render ${input} first (npm run render:${variant})`);

const pkg = `@remotion/compositor-${process.platform}-${process.arch}${process.platform === "win32" ? "-msvc" : ""}`;
const dir = path.dirname(require.resolve(`${pkg}/package.json`));
const ffmpeg = readdirSync(dir).find((f) => /^ffmpeg(\.exe)?$/.test(f));
if (!ffmpeg) throw new Error(`no ffmpeg in ${dir}`);

mkdirSync("out/frames", { recursive: true });
for (let t = 1; t < 34; t += 2) {
  const outFile = path.resolve(`out/frames/${variant}-${String(t).padStart(2, "0")}s.png`);
  const r = spawnSync(path.join(dir, ffmpeg), ["-hide_banner", "-loglevel", "error", "-y", "-ss", String(t), "-i", input, "-frames:v", "1", outFile], {
    stdio: "inherit",
    cwd: dir,
  });
  if (r.status !== 0) throw new Error(`ffmpeg failed at t=${t}`);
}
console.log("wrote out/frames/");
