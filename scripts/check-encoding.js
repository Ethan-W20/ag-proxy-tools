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

let checkedFiles = 0;
const problems = [];

// Stored as base64 to keep this script itself free from visible mojibake literals.
const mojibakeFragmentBase64 = [
  "55KH6ZSL55yw",
  "5r626L6r6Kem",
  "6Y2d5baF57Cy",
  "55KQ77mA5b2/",
];

const mojibakeFragments = mojibakeFragmentBase64.map((item) =>
  Buffer.from(item, "base64").toString("utf8"),
);

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

    checkedFiles += 1;
    const content = fs.readFileSync(fullPath, "utf8");
    const relativePath = path.relative(rootDir, fullPath);

    if (!content.includes("\uFFFD")) {
      const skipFragmentCheck =
        relativePath === path.join("scripts", "check-encoding.js") ||
        relativePath === path.join("scripts", "apply-known-mojibake-fixes.js");

      const fragment = skipFragmentCheck
        ? null
        : mojibakeFragments.find((frag) => content.includes(frag));

      if (!fragment) {
        continue;
      }

      const lines = content.split(/\r?\n/);
      lines.forEach((line, index) => {
        if (line.includes(fragment)) {
          problems.push({
            file: relativePath,
            line: index + 1,
            preview: line.trim().slice(0, 120),
            reason: `mojibake fragment: ${fragment}`,
          });
        }
      });
      continue;
    }

    const lines = content.split(/\r?\n/);
    lines.forEach((line, index) => {
      if (line.includes("\uFFFD")) {
        problems.push({
          file: relativePath,
          line: index + 1,
          preview: line.trim().slice(0, 120),
          reason: "replacement char U+FFFD",
        });
      }
    });
  }
}

walk(rootDir);

if (problems.length > 0) {
  console.error("Encoding check failed: detected corrupted text fragments.");
  problems.slice(0, 120).forEach((item) => {
    const reason = item.reason ? ` [${item.reason}]` : "";
    console.error(`- ${item.file}:${item.line}${reason} ${item.preview}`);
  });
  if (problems.length > 120) {
    console.error(`... and ${problems.length - 120} more issue(s).`);
  }
  process.exit(1);
}

console.log(`Encoding check passed. Scanned ${checkedFiles} text files.`);
