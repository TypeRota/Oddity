#!/usr/bin/env node
// 游戏包 UI 只依赖包内代码、@packages 别名与宿主依赖树或约定的公共入口,
// 不引用宿主内部实现, 也不直接使用 Node API.
// 宿主 UI 只有 src/host/generated 可以引用游戏包, 其余文件不得直接依赖包路径.
// 静态扫描, 不执行包代码; 命中时输出 文件:行:列, 便于直接定位.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { builtinModules } from "node:module";
import { dirname, extname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const packagesRoot = join(root, "launcher/packages");
const hostRoot = join(root, "launcher/ui");
const hostApiRoot = join(hostRoot, "src/host-api");
const hostSourceRoot = join(hostRoot, "src");
const generatedRoot = join(hostSourceRoot, "host/generated");
const hostDependenciesRoot = join(hostRoot, "node_modules");
const nodeBuiltins = new Set(builtinModules);

const sourceExtensions = new Set([
  ".ts",
  ".tsx",
  ".mts",
  ".cts",
  ".js",
  ".jsx",
  ".mjs",
  ".cjs",
  ".vue",
  ".css",
]);

// 覆盖静态导入、再导出、动态导入、require 与 CSS @import.
const importPatterns = [
  /(?<!@)\bimport\s+(?:type\s+)?(?:[^;"']*?\sfrom\s+)?["']([^"']+)["']/g,
  /\bexport\s+(?:type\s+)?(?:[^;"']*?\sfrom\s+)?["']([^"']+)["']/g,
  /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
  /\brequire\s*\(\s*["']([^"']+)["']\s*\)/g,
  /@import\s+(?:url\(\s*)?["']([^"']+)["']/g,
];

const scriptStyleBlock =
  /<\s*(script|style)\b[^>]*>([\s\S]*?)<\s*\/\s*\1\s*>/gi;

function isInside(parent, child) {
  return child === parent || child.startsWith(parent + sep);
}

// 包 UI 与宿主共用一份依赖树; pnpm 只在宿主 node_modules 下建立直接依赖链接.
function hostProvides(spec) {
  // 依赖未安装时无法判断, 交给后续构建报错.
  if (!existsSync(hostDependenciesRoot)) return true;
  const [first, second] = spec.split("/");
  const name = spec.startsWith("@") ? `${first}/${second}` : first;
  return existsSync(join(hostDependenciesRoot, name));
}

// 注释只做遮蔽, 保持长度与换行不变, 让命中位置仍能换算成原始行号.
function maskComments(source) {
  let masked = "";
  let index = 0;
  while (index < source.length) {
    const char = source[index];
    const next = source[index + 1];
    if (char === "/" && next === "/") {
      const stop = source.indexOf("\n", index);
      const end = stop === -1 ? source.length : stop;
      masked += " ".repeat(end - index);
      index = end;
    } else if (char === "/" && next === "*") {
      const stop = source.indexOf("*/", index + 2);
      const end = stop === -1 ? source.length : stop + 2;
      masked += source.slice(index, end).replace(/[^\n]/g, " ");
      index = end;
    } else if (char === '"' || char === "'" || char === "`") {
      let cursor = index + 1;
      while (cursor < source.length) {
        if (source[cursor] === "\\") cursor += 2;
        else if (source[cursor] === char) {
          cursor += 1;
          break;
        } else cursor += 1;
      }
      masked += source.slice(index, cursor);
      index = cursor;
    } else {
      masked += char;
      index += 1;
    }
  }
  return masked;
}

function locate(text, offset) {
  let line = 1;
  let lineStart = 0;
  for (let index = 0; index < offset; index += 1) {
    if (text[index] === "\n") {
      line += 1;
      lineStart = index + 1;
    }
  }
  return { line, column: offset - lineStart + 1 };
}

function segmentsOf(file, text) {
  if (!file.endsWith(".vue")) return [{ start: 0, content: text }];
  const segments = [];
  for (const match of text.matchAll(scriptStyleBlock)) {
    const start = match.index + match[0].indexOf(">") + 1;
    segments.push({ start, content: match[2] });
  }
  return segments;
}

// 返回 null 表示允许, 否则返回拒绝原因.
function violationOf(file, packageRoot, spec) {
  const clean = spec.replace(/[?#].*$/, "");

  // 宿主公共入口只认 src/host-api; 越过该目录的 @host 子路径同样拒绝.
  if (clean === "@host" || clean.startsWith("@host/")) {
    const target = resolve(
      hostApiRoot,
      clean === "@host" ? "" : clean.slice("@host/".length),
    );
    return isInside(hostApiRoot, target) ? null : "@host 越出宿主公共入口";
  }
  if (clean === "@packages" || clean.startsWith("@packages/")) {
    const target = resolve(packagesRoot, clean.slice("@packages".length + 1));
    return isInside(packagesRoot, target)
      ? null
      : "包别名越出 launcher/packages";
  }
  if (clean.startsWith(".")) {
    const target = resolve(dirname(file), clean);
    if (isInside(hostRoot, target)) return "引用宿主内部实现";
    if (!isInside(packageRoot, target)) return "相对导入越出当前游戏包";
    return null;
  }
  if (clean.startsWith("/")) return "绝对路径导入指向宿主";
  if (/(^|\/)launcher\/ui(\/|$)/.test(clean)) return "引用宿主内部实现";
  if (clean.startsWith("node:") || nodeBuiltins.has(clean))
    return "不得使用 Node API";
  if (!hostProvides(clean)) return "宿主未提供该依赖, 也未经允许";
  return null;
}

// 宿主只能经生成物引用游戏包; 手写文件直接 import 包路径会让宿主重新依赖具体游戏.
function hostViolationOf(file, spec) {
  const clean = spec.replace(/[?#].*$/, "");
  if (clean !== "@packages" && !clean.startsWith("@packages/")) return null;
  return isInside(generatedRoot, file)
    ? null
    : "宿主只能经 src/host/generated 引用游戏包";
}

function collectFiles(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === "node_modules" || entry.name === "dist") continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...collectFiles(path));
    else if (entry.isFile() && sourceExtensions.has(extname(path)))
      files.push(path);
  }
  return files;
}

function findPackageUiDirs() {
  if (!existsSync(packagesRoot)) return [];
  return readdirSync(packagesRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(packagesRoot, entry.name, "ui"))
    .filter((directory) => existsSync(join(directory, "src")));
}

const violations = [];
const seen = new Set();

// 同一份扫描逻辑复用于包 UI 与宿主 UI, 只是拒绝规则不同.
function scanFiles(files, check) {
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    for (const segment of segmentsOf(file, text)) {
      const masked = maskComments(segment.content);
      for (const pattern of importPatterns) {
        for (const match of masked.matchAll(pattern)) {
          const spec = match[1];
          const reason = check(file, spec);
          if (!reason) continue;
          const offset =
            segment.start + match.index + match[0].lastIndexOf(spec);
          const { line, column } = locate(text, offset);
          const key = `${file}:${line}:${column}:${spec}:${reason}`;
          if (seen.has(key)) continue;
          seen.add(key);
          violations.push({
            file: relative(root, file).split(sep).join("/"),
            line,
            column,
            spec,
            reason,
          });
        }
      }
    }
  }
}

const packageUiDirs = findPackageUiDirs();
let packageFiles = 0;
for (const uiDir of packageUiDirs) {
  const packageRoot = dirname(uiDir);
  const files = collectFiles(uiDir);
  packageFiles += files.length;
  scanFiles(files, (file, spec) => violationOf(file, packageRoot, spec));
}

const hostFiles = existsSync(hostSourceRoot)
  ? collectFiles(hostSourceRoot)
  : [];
scanFiles(hostFiles, hostViolationOf);

if (violations.length > 0) {
  console.error(`UI 边界检查失败 (${violations.length} 项):`);
  for (const violation of violations) {
    console.error(
      `- ${violation.file}:${violation.line}:${violation.column} ` +
        `引用 ${violation.spec} — ${violation.reason}`,
    );
  }
  process.exit(1);
}

console.log(
  `UI 边界检查通过: ${packageUiDirs.length} 个包 ${packageFiles} 个文件, ` +
    `宿主 ${hostFiles.length} 个文件`,
);
