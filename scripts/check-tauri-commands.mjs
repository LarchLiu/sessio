import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";

const root = process.cwd();
const rustRoot = path.join(root, "src-tauri", "src");
const frontendRoot = path.join(root, "src");

const targetOs =
  process.env.SESSIO_COMMAND_TARGET_OS ??
  ({ darwin: "macos", win32: "windows" }[process.platform] ?? process.platform);
const targetFamily = targetOs === "windows" ? "windows" : "unix";

function walk(dir, predicate) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = path.join(dir, entry);
    const stat = statSync(full);
    if (stat.isDirectory()) {
      if (entry !== "target" && entry !== "node_modules" && entry !== ".git") {
        out.push(...walk(full, predicate));
      }
    } else if (predicate(full)) {
      out.push(full);
    }
  }
  return out;
}

function splitTopLevel(input) {
  const parts = [];
  let depth = 0;
  let start = 0;
  let quote = null;
  for (let i = 0; i < input.length; i += 1) {
    const char = input[i];
    if (quote) {
      if (char === quote && input[i - 1] !== "\\") quote = null;
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
    } else if (char === "(") {
      depth += 1;
    } else if (char === ")") {
      depth -= 1;
    } else if (char === "," && depth === 0) {
      parts.push(input.slice(start, i).trim());
      start = i + 1;
    }
  }
  parts.push(input.slice(start).trim());
  return parts.filter(Boolean);
}

function evaluateCfg(expr) {
  const value = expr.trim();
  const call = value.match(/^(any|all|not)\((.*)\)$/s);
  if (call) {
    const [, op, body] = call;
    const parts = splitTopLevel(body).map(evaluateCfg);
    if (op === "any") return parts.some(Boolean);
    if (op === "all") return parts.every(Boolean);
    return !parts[0];
  }

  const targetOsMatch = value.match(/^target_os\s*=\s*"([^"]+)"$/);
  if (targetOsMatch) return targetOs === targetOsMatch[1];

  const targetFamilyMatch = value.match(/^target_family\s*=\s*"([^"]+)"$/);
  if (targetFamilyMatch) return targetFamily === targetFamilyMatch[1];

  if (value === "windows") return targetOs === "windows";
  if (value === "unix") return targetFamily === "unix";
  if (value === "macos") return targetOs === "macos";
  if (value === "linux") return targetOs === "linux";

  return true;
}

function attrsAreActive(attrs) {
  return attrs.every(attr => {
    const cfg = attr.match(/^#\s*\[\s*cfg\((.*)\)\s*\]\s*$/s);
    return cfg ? evaluateCfg(cfg[1]) : true;
  });
}

function collectRustCommands(files) {
  const commands = [];
  const duplicates = new Map();
  const byName = new Map();

  for (const file of files) {
    const rel = path.relative(root, file);
    const lines = readFileSync(file, "utf8").split(/\r?\n/);
    let attrs = [];
    for (let index = 0; index < lines.length; index += 1) {
      const trimmed = lines[index].trim();
      if (trimmed.startsWith("#[")) {
        attrs.push(trimmed);
        continue;
      }
      if (trimmed === "" || trimmed.startsWith("//")) {
        continue;
      }

      const fnMatch = trimmed.match(/^(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b/);
      if (fnMatch && attrs.some(attr => attr.includes("tauri::command")) && attrsAreActive(attrs)) {
        const record = { name: fnMatch[1], file: rel, line: index + 1 };
        commands.push(record);
        const existing = byName.get(record.name);
        if (existing) duplicates.set(record.name, [existing, record]);
        byName.set(record.name, record);
      }
      attrs = [];
    }
  }

  return { commands, duplicates, byName };
}

function stripLineComments(input) {
  return input
    .split(/\r?\n/)
    .map(line => line.replace(/\/\/.*$/, ""))
    .join("\n");
}

function extractGenerateHandlerEntries(libRs) {
  const source = readFileSync(libRs, "utf8");
  const marker = "tauri::generate_handler![";
  const start = source.indexOf(marker);
  if (start === -1) {
    throw new Error("Could not find tauri::generate_handler![...] in src-tauri/src/lib.rs");
  }
  let depth = 0;
  let end = -1;
  const bodyStart = start + marker.length;
  for (let i = bodyStart; i < source.length; i += 1) {
    const char = source[i];
    if (char === "[") depth += 1;
    if (char === "]") {
      if (depth === 0) {
        end = i;
        break;
      }
      depth -= 1;
    }
  }
  if (end === -1) {
    throw new Error("Could not parse generate_handler body");
  }

  return stripLineComments(source.slice(bodyStart, end))
    .split(",")
    .map(entry => entry.trim())
    .filter(Boolean)
    .map(pathName => ({
      pathName,
      name: pathName.split("::").at(-1),
    }));
}

function collectFrontendInvokes(files) {
  const invokes = [];
  const invokePattern = /invoke(?:\s*<[^>()]*>)?\s*\(\s*(["'`])([^"'`]+)\1/g;
  for (const file of files) {
    const rel = path.relative(root, file);
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(invokePattern)) {
      const before = source.slice(0, match.index);
      const line = before.split(/\r?\n/).length;
      invokes.push({ name: match[2], file: rel, line });
    }
  }
  return invokes;
}

const rustFiles = walk(rustRoot, file => file.endsWith(".rs"));
const frontendFiles = walk(frontendRoot, file => /\.(ts|tsx)$/.test(file));
const { commands, duplicates: commandDuplicates, byName: commandsByName } =
  collectRustCommands(rustFiles);
const handlers = extractGenerateHandlerEntries(path.join(rustRoot, "lib.rs"));
const handlerNames = new Set(handlers.map(entry => entry.name));
const invokes = collectFrontendInvokes(frontendFiles);

const errors = [];
const handlerCounts = new Map();
for (const handler of handlers) {
  handlerCounts.set(handler.name, (handlerCounts.get(handler.name) ?? 0) + 1);
  if (!commandsByName.has(handler.name)) {
    errors.push(`handler has no active #[tauri::command]: ${handler.pathName}`);
  }
}
for (const [name, count] of handlerCounts) {
  if (count > 1) errors.push(`duplicate handler registration: ${name} (${count}x)`);
}
for (const [name, records] of commandDuplicates) {
  errors.push(
    `duplicate active command definition: ${name} at ${records
      .map(record => `${record.file}:${record.line}`)
      .join(", ")}`,
  );
}
for (const command of commands) {
  if (!handlerNames.has(command.name)) {
    errors.push(`active command is not registered: ${command.name} at ${command.file}:${command.line}`);
  }
}
for (const invoke of invokes) {
  if (!handlerNames.has(invoke.name)) {
    errors.push(`frontend invoke has no handler: ${invoke.name} at ${invoke.file}:${invoke.line}`);
  }
}

if (errors.length > 0) {
  console.error(`Tauri command check failed for target_os=${targetOs}`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

console.log(
  `Tauri command check passed for target_os=${targetOs}: ${commands.length} commands, ${handlers.length} handlers, ${new Set(invokes.map(invoke => invoke.name)).size} frontend invoke names.`,
);
