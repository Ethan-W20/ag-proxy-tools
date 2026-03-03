import fs from "node:fs";
import path from "node:path";

const rootDir = process.cwd();
const indexPath = path.join(rootDir, "src", "index.html");
const mainPath = path.join(rootDir, "src", "main.js");

const requiredHtmlIds = [
  "navSettingsLabel",
  "settingsPageTitle",
  "settingsPageDesc",
  "settingsTransportTitle",
  "settingsAppearanceTitle",
  "themeSettingLabel",
  "themeSettingDesc",
  "themeDarkText",
  "themeLightText",
  "themeSystemText",
  "uiLanguageLabel",
  "uiLanguageDesc",
  "uiLangBtnZh",
  "uiLangBtnEn",
];

const requiredMainSymbols = [
  "UI_LANGUAGE_KEY",
  "UI_STATIC_TEXT",
  "setUiLanguage(",
  "restoreUiLanguage(",
  "applyUiLanguage(",
];

function readFileSafe(filePath) {
  if (!fs.existsSync(filePath)) {
    return "";
  }
  return fs.readFileSync(filePath, "utf8");
}

function collectChineseLines(filePath, content) {
  const lines = content.split(/\r?\n/);
  const findings = [];
  lines.forEach((line, index) => {
    const hasCjk = /[\u4e00-\u9fff]/.test(line);
    if (!hasCjk) {
      return;
    }
    const isComment =
      line.trimStart().startsWith("//") ||
      line.trimStart().startsWith("/*") ||
      line.trimStart().startsWith("*") ||
      line.trimStart().startsWith("<!--");
    if (isComment) {
      return;
    }
    const isCovered =
      line.includes("uiText(") ||
      line.includes("UI_STATIC_TEXT") ||
      line.includes("id=\"uiLangBtn") ||
      line.includes("setUiLanguage(");
    if (!isCovered) {
      findings.push({
        file: path.relative(rootDir, filePath),
        line: index + 1,
        text: line.trim().slice(0, 160),
      });
    }
  });
  return findings;
}

const indexContent = readFileSafe(indexPath);
const mainContent = readFileSafe(mainPath);

const missingHtmlIds = requiredHtmlIds.filter(
  (id) => !indexContent.includes(`id="${id}"`),
);
const missingMainSymbols = requiredMainSymbols.filter(
  (symbol) => !mainContent.includes(symbol),
);

const uncoveredChinese = [
  ...collectChineseLines(indexPath, indexContent),
  ...collectChineseLines(mainPath, mainContent),
];

const summary = {
  ok: missingHtmlIds.length === 0 && missingMainSymbols.length === 0,
  missing_html_ids: missingHtmlIds,
  missing_main_symbols: missingMainSymbols,
  uncovered_chinese_line_count: uncoveredChinese.length,
  uncovered_chinese_sample: uncoveredChinese.slice(0, 60),
};

const outDir = path.join(rootDir, "docs", "maintenance");
fs.mkdirSync(outDir, { recursive: true });
fs.writeFileSync(
  path.join(outDir, "i18n-last-report.json"),
  `${JSON.stringify(summary, null, 2)}\n`,
  "utf8",
);

if (!summary.ok) {
  console.error("i18n structural check failed.");
  if (missingHtmlIds.length) {
    console.error(`missing html ids: ${missingHtmlIds.join(", ")}`);
  }
  if (missingMainSymbols.length) {
    console.error(`missing main symbols: ${missingMainSymbols.join(", ")}`);
  }
  process.exit(1);
}

if (uncoveredChinese.length > 0) {
  console.warn(
    `i18n coverage warning: ${uncoveredChinese.length} Chinese lines not mapped yet.`,
  );
}

console.log("i18n structural check passed.");
console.log(`uncovered Chinese lines: ${uncoveredChinese.length}`);
