import fs from "node:fs";
import path from "node:path";

const rootDir = process.cwd();

const textExtensions = new Set([
  ".rs",
  ".js",
  ".mjs",
  ".cjs",
  ".ts",
  ".tsx",
  ".jsx",
  ".html",
  ".css",
  ".scss",
  ".json",
  ".md",
  ".toml",
  ".yml",
  ".yaml",
  ".txt",
]);

const skipDirs = new Set([
  ".git",
  "node_modules",
  "dist",
  "build",
  "target",
  ".ai-workspace",
]);

const replacements = new Map([
  ["供应商配置已保存", "供应商配置已保存"],
  ["转发到供应商", "转发到供应商"],
  ["供应商请求失败", "供应商请求失败"],
  ["供应商返回错误", "供应商返回错误"],
  ["供应商响应完成", "供应商响应完成"],
  ["返回 SSE 响应给", "返回 SSE 响应给"],
  ["鉴权/限流", "鉴权/限流"],
]);

let scanned = 0;
let changed = 0;
const touched = [];

function walk(dir) {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (!skipDirs.has(entry.name)) {
        walk(fullPath);
      }
      continue;
    }
    if (!entry.isFile()) {
      continue;
    }
    if (!textExtensions.has(path.extname(entry.name).toLowerCase())) {
      continue;
    }
    scanned += 1;
    const original = fs.readFileSync(fullPath, "utf8");
    let next = original;
    for (const [from, to] of replacements.entries()) {
      if (next.includes(from)) {
        next = next.split(from).join(to);
      }
    }
    if (next !== original) {
      fs.writeFileSync(fullPath, next, "utf8");
      changed += 1;
      touched.push(path.relative(rootDir, fullPath));
    }
  }
}

walk(rootDir);

console.log(`mojibake autofix scanned: ${scanned}`);
console.log(`mojibake autofix changed files: ${changed}`);
if (touched.length) {
  touched.slice(0, 100).forEach((file) => console.log(`- ${file}`));
}
