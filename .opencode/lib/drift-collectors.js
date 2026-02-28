const fs = require("node:fs/promises");
const path = require("node:path");

const CODE_EXTENSIONS = new Set([
  ".js",
  ".jsx",
  ".ts",
  ".tsx",
  ".py",
  ".go",
  ".rs",
  ".java",
  ".kt",
  ".rb",
  ".php",
  ".cs",
  ".swift",
  ".c",
  ".cc",
  ".cpp",
  ".h",
  ".hpp",
]);

async function pathExists(targetPath) {
  try {
    await fs.access(targetPath);
    return true;
  } catch {
    return false;
  }
}

async function walkFiles(rootDir, includeFile) {
  const results = [];
  const stack = [rootDir];

  while (stack.length > 0) {
    const current = stack.pop();
    const entries = await fs.readdir(current, { withFileTypes: true });

    for (const entry of entries) {
      if (entry.name === ".git" || entry.name === "node_modules") {
        continue;
      }

      const fullPath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(fullPath);
        continue;
      }

      if (entry.isFile() && includeFile(fullPath)) {
        results.push(fullPath);
      }
    }
  }

  return results;
}

function countMatches(content, expression) {
  const matches = content.match(expression);
  return matches ? matches.length : 0;
}

async function collectContextData(cwd) {
  const contextDir = path.join(cwd, "context");
  const contextExists = await pathExists(contextDir);

  if (!contextExists) {
    return {
      exists: false,
      fileCount: 0,
      files: {},
      checkboxes: {
        total: 0,
        checked: 0,
        unchecked: 0,
      },
      lastModified: null,
      structure: [],
    };
  }

  const markdownFiles = await walkFiles(contextDir, (fullPath) =>
    fullPath.endsWith(".md")
  );

  const files = {};
  const structure = [];
  let total = 0;
  let checked = 0;
  let unchecked = 0;
  let newestMtimeMs = 0;

  for (const fullPath of markdownFiles) {
    const relativePath = path.relative(cwd, fullPath);
    const content = await fs.readFile(fullPath, "utf8");
    const stats = await fs.stat(fullPath);
    const lineCount = content.split("\n").length;

    const fileUnchecked = countMatches(content, /- \[ \]/g);
    const fileChecked = countMatches(content, /- \[[xX]\]/g);

    total += fileUnchecked + fileChecked;
    checked += fileChecked;
    unchecked += fileUnchecked;

    if (stats.mtimeMs > newestMtimeMs) {
      newestMtimeMs = stats.mtimeMs;
    }

    files[relativePath] = {
      lines: lineCount,
      lastModified: stats.mtime.toISOString(),
      hasCheckboxes: fileUnchecked + fileChecked > 0,
      checkboxCounts: {
        checked: fileChecked,
        unchecked: fileUnchecked,
      },
    };
    structure.push(relativePath);
  }

  structure.sort();

  return {
    exists: true,
    fileCount: markdownFiles.length,
    files,
    checkboxes: {
      total,
      checked,
      unchecked,
    },
    lastModified: newestMtimeMs ? new Date(newestMtimeMs).toISOString() : null,
    structure,
  };
}

async function detectFrameworks(cwd) {
  const frameworks = [];
  const packageJsonPath = path.join(cwd, "package.json");

  if (await pathExists(packageJsonPath)) {
    frameworks.push("node");
    try {
      const raw = await fs.readFile(packageJsonPath, "utf8");
      const pkg = JSON.parse(raw);
      const deps = {
        ...(pkg.dependencies || {}),
        ...(pkg.devDependencies || {}),
      };

      if (deps.react) frameworks.push("react");
      if (deps.next) frameworks.push("nextjs");
      if (deps.vue) frameworks.push("vue");
      if (deps.svelte) frameworks.push("svelte");
      if (deps.express) frameworks.push("express");
    } catch {
      frameworks.push("package-json-unreadable");
    }
  }

  if (await pathExists(path.join(cwd, "pyproject.toml"))) frameworks.push("python");
  if (await pathExists(path.join(cwd, "go.mod"))) frameworks.push("go");
  if (await pathExists(path.join(cwd, "Cargo.toml"))) frameworks.push("rust");

  return Array.from(new Set(frameworks));
}

async function collectCodeData(cwd) {
  const scanDirs = ["src", "lib", "app", "server", "backend", "packages"];
  const structure = [];
  const byExtension = {};
  let fileCount = 0;

  for (const dir of scanDirs) {
    const fullDir = path.join(cwd, dir);
    if (!(await pathExists(fullDir))) {
      continue;
    }

    const files = await walkFiles(fullDir, (fullPath) =>
      CODE_EXTENSIONS.has(path.extname(fullPath).toLowerCase())
    );

    for (const fullPath of files) {
      const ext = path.extname(fullPath).toLowerCase() || "<none>";
      byExtension[ext] = (byExtension[ext] || 0) + 1;
      structure.push(path.relative(cwd, fullPath));
      fileCount += 1;
    }
  }

  structure.sort();

  return {
    frameworks: await detectFrameworks(cwd),
    scannedRoots: scanDirs,
    structure,
    fileCount,
    byExtension,
  };
}

async function collectAll(cwd, options = {}) {
  const sources = Array.isArray(options.sources) && options.sources.length > 0
    ? options.sources
    : ["context", "code"];

  return {
    context: sources.includes("context") ? await collectContextData(cwd) : null,
    code: sources.includes("code") ? await collectCodeData(cwd) : null,
  };
}

module.exports = {
  collectAll,
  collectContextData,
  collectCodeData,
};
