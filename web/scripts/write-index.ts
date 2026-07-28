import { mkdir, readdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";

const outDir = resolve("dist");
const assetsDir = join(outDir, "assets");
const files = await readdir(assetsDir);
const jsFile = files.find((file) => file.startsWith("app.") && file.endsWith(".js"));

if (!jsFile) {
  throw new Error(`No app bundle found in ${assetsDir}`);
}

await mkdir(outDir, { recursive: true });
await writeFile(
  join(outDir, "index.html"),
  `<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <meta name="theme-color" content="#eef3f7" />
  <meta name="color-scheme" content="light dark" />
  <title>AI Gateway</title>
  <link rel="stylesheet" href="/assets/styles.css" />
</head>
<body>
  <div id="root"></div>
  <script type="module" src="/assets/${jsFile}"></script>
</body>
</html>
`,
);
