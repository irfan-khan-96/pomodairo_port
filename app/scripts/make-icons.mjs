// Regenerates every app icon from the SVG sources in src-tauri/icons/source.
//
//   npm run icons
//
// icon.svg is used for 48px and up; icon-small.svg (a simplified tomato) for
// the 16-32px slots, where the segment design turns to mush.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Resvg } from "@resvg/resvg-js";

const icons = new URL("../src-tauri/icons/", import.meta.url);
const large = readFileSync(new URL("source/icon.svg", icons));
const small = readFileSync(new URL("source/icon-small.svg", icons));

const render = (svg, size) => new Resvg(svg, { fitTo: { mode: "width", value: size } }).render().asPng();

// 1. Let Tauri produce the full set (ico, icns, pngs) from the large design.
const tmp = mkdtempSync(join(tmpdir(), "pomodairo-icons-"));
const master = join(tmp, "icon-1024.png");
writeFileSync(master, render(large, 1024));
const tauriCli = fileURLToPath(new URL("../node_modules/@tauri-apps/cli/tauri.js", import.meta.url));
execFileSync(process.execPath, [tauriCli, "icon", master], { stdio: "inherit" });
rmSync(tmp, { recursive: true });
// Desktop-only app: drop the mobile icon sets the CLI also writes.
for (const dir of ["android", "ios"]) rmSync(new URL(dir, icons), { recursive: true, force: true });

// 2. Swap in the small design where it matters.
writeFileSync(new URL("32x32.png", icons), render(small, 32));

/** ICO file with PNG-compressed entries (supported since Windows Vista). */
function ico(entries) {
  const header = Buffer.alloc(6 + 16 * entries.length);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(entries.length, 4);
  let offset = header.length;
  entries.forEach(({ size, png }, i) => {
    const at = 6 + 16 * i;
    header.writeUInt8(size >= 256 ? 0 : size, at); // 0 means 256
    header.writeUInt8(size >= 256 ? 0 : size, at + 1);
    header.writeUInt8(0, at + 2); // palette size
    header.writeUInt8(0, at + 3); // reserved
    header.writeUInt16LE(1, at + 4); // colour planes
    header.writeUInt16LE(32, at + 6); // bits per pixel
    header.writeUInt32LE(png.length, at + 8);
    header.writeUInt32LE(offset, at + 12);
    offset += png.length;
  });
  return Buffer.concat([header, ...entries.map((e) => e.png)]);
}

const sizes = [16, 24, 32, 48, 64, 256];
writeFileSync(
  new URL("icon.ico", icons),
  ico(sizes.map((size) => ({ size, png: render(size <= 32 ? small : large, size) }))),
);
console.log("Icons regenerated (small design used for 16-32px).");
