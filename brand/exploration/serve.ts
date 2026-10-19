// Local preview of the showcase page.
// Usage: bun brand/exploration/serve.ts [port]
// index.html is authored without a document skeleton (the artifact host adds one), so wrap it here.
import { join } from "node:path";

const dir = import.meta.dir;
const port = Number(process.argv[2] ?? 4800);

Bun.serve({
  port,
  async fetch(req) {
    const path = new URL(req.url).pathname;
    const name = path === "/" ? "index.html" : path === "/round2" ? "round2.html" : path.replace(/^\/+/, "");
    if (name.endsWith(".html") && !name.includes("..") && (await Bun.file(join(dir, name)).exists())) {
      const page = await Bun.file(join(dir, name)).text();
      return new Response(
        `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"></head><body>${page}</body></html>`,
        { headers: { "content-type": "text/html; charset=utf-8" } },
      );
    }
    const file = Bun.file(join(dir, path.replace(/^\/+/, "")));
    if (!path.includes("..") && (await file.exists())) return new Response(file);
    return new Response("Not found", { status: 404 });
  },
});
console.log(`Linksy showcase on http://localhost:${port} (round 2: /round2)`);
