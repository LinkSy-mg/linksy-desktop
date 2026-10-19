// Export every mark × palette to standalone SVG files.
// Usage: bun brand/exploration/export.ts [marks file] [out subdir]
//   e.g. bun brand/exploration/export.ts marks-r2.js svg-r2
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

await import("./marks.js");
const [marksFile = "marks.js", sub = "svg"] = process.argv.slice(2);
if (marksFile !== "marks.js") await import(`./${marksFile}`);
const { marks, palettes } = (globalThis as any).LinksyMarks;

const outDir = join(import.meta.dir, sub);
mkdirSync(outDir, { recursive: true });

for (const p of palettes) {
  const a = p.id === "mono" ? "#141821" : p.a;
  const b = p.id === "mono" ? "#141821" : p.b;
  const c = p.id === "mono" ? "#141821" : p.c;
  const style = `.a{fill:${a}}.b{fill:${b}}.c{fill:${c}}.m{fill:${a};opacity:.18}.sa{fill:none;stroke:${a}}.sb{fill:none;stroke:${b}}`;
  for (const m of marks) {
    const body = m.svg(m.id).replaceAll("var(--a)", a).replaceAll("var(--b)", b);
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="512" height="512">\n  <style>${style}</style>${body}\n</svg>\n`;
    writeFileSync(join(outDir, `${m.id}--${p.id}.svg`), svg);
  }
}
console.log(`wrote ${marks.length * palettes.length} files to ${outDir}`);
