import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { LOCAL_CLIENT_TARGETS, LOCAL_CLIENT_CONFIG_MESSAGES, renderLocalClientConfiguration, localClientVerificationPrompt, localClientUsageGuidance } from "../src/local-client-config.js";

const name = "pcp-cb9876ba-a25f-4478-a482-af3af7400872";
const server = { command: "/Users/测试 person's PCP/bin/pcp-mcp", args: ["a b", 'quote"', "line\nnext", "'$HOME`printf bad`$(printf bad)"], env: {
  PCP_CLIENT_ID: "client:local:independent", PCP_ENROLLMENT_FILE: "/Users/测试 person's PCP/enrollment.json",
  PCP_MCP_SURFACE: "generic", PCP_MCP_TOOLSET: "context",
} };
const config = { mcpServers: { [name]: server } };

function jsonLaunch(target) {
  const artifact = renderLocalClientConfiguration(config, target), parsed = JSON.parse(artifact.text);
  if (target === "vscode") { const { type, ...entry } = parsed.servers[name]; assert.equal(type, "stdio"); return entry; }
  if (target.startsWith("opencode")) {
    const entry = (target === "opencode-v2" ? parsed.mcp.servers : parsed.mcp)[name];
    assert.equal(entry.type, "local");
    assert.equal(Object.hasOwn(entry, "enabled"), false); assert.equal(Object.hasOwn(entry, "disabled"), false);
    return { command: entry.command[0], args: entry.command.slice(1), env: entry.environment };
  }
  return parsed.mcpServers[name];
}

test("every JSON client preserves command, argument boundaries and private enrollment reference", () => {
  const before = JSON.stringify(config);
  for (const target of LOCAL_CLIENT_TARGETS.filter(item => item.format === "json")) {
    assert.deepEqual(jsonLaunch(target.id), server, target.id);
    const artifact = renderLocalClientConfiguration(config, target.id);
    assert.equal(artifact.mimeType, "application/json"); assert.match(artifact.filename, /\.json$/);
    assert.doesNotMatch(artifact.text, /autoApprove|alwaysAllow|"trust"/);
  }
  assert.equal(JSON.stringify(config), before);
});

test("Codex exports TOML tables and DSH exports a complete insert overlay with a lossless short namespace", () => {
  const codex = renderLocalClientConfiguration(config, "codex");
  assert.match(codex.text, /^\[mcp_servers\."pcp-/);
  assert.match(codex.text, /\.env\]/); assert.match(codex.filename, /\.toml$/);
  const dsh = renderLocalClientConfiguration(config, "dsh");
  assert.match(dsh.text, /^- insert:/); assert.match(dsh.text, /@deepseek-ai\/dsh-mcp-client/);
  assert.ok(dsh.serverName.length <= 32); assert.match(dsh.serverName, /^[A-Za-z0-9_-]+$/);
  const decoded = [...dsh.serverName.slice(4)].reduce((acc, digit) => acc * 36n + BigInt(parseInt(digit, 36)), 0n);
  assert.equal(decoded.toString(16).padStart(32, "0"), name.slice(4).replaceAll("-", ""));
  assert.match(dsh.installCommand, /^dsh web --patch '\.\/pcp-/);
  assert.ok(dsh.installCommand.includes(dsh.filename));
});

test("Claude install command passes literal JSON through a POSIX shell without expanding values", () => {
  const directory = mkdtempSync(join(tmpdir(), "pcp-export-"));
  try {
    const capture = join(directory, "args.json"), executable = join(directory, "claude");
    writeFileSync(executable, "#!/usr/bin/env python3\nimport json,os,sys\nwith open(os.environ['PCP_ARG_CAPTURE'],'w') as out: json.dump(sys.argv[1:],out)\n", { mode: 0o700 });
    const artifact = renderLocalClientConfiguration(config, "claude-code");
    const result = spawnSync("/bin/sh", ["-c", artifact.installCommand], { encoding: "utf8", env: { PATH: directory + ":" + process.env.PATH, PCP_ARG_CAPTURE: capture } });
    assert.equal(result.status, 0, result.stderr);
    const args = JSON.parse(readFileSync(capture, "utf8"));
    assert.deepEqual(args.slice(0, 5), ["mcp", "add-json", "--scope", "user", name]);
    assert.deepEqual(JSON.parse(args[5]), server);
    assert.equal(args.length, 6);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test("unknown formats and malformed process specifications fail instead of producing partial configuration", () => {
  assert.throws(() => renderLocalClientConfiguration(config, "guess-client"), /Unknown/);
  for (const broken of [{}, { mcpServers: {} }, { mcpServers: { a: server, b: server } },
    { mcpServers: { a: { ...server, args: "--bad" } } }, { mcpServers: { a: { ...server, env: { ...server.env, PCP_CLIENT_ID: null } } } }])
    assert.throws(() => renderLocalClientConfiguration(broken), /Expected|Invalid/);
});

test("verification uses the expected identity and exact scopes; read guidance does not grant writes", () => {
  const profile = { principalId: "client:local:abc", scopes: ["project:one", "user:self"] };
  const prompt = localClientVerificationPrompt("pcp_one", profile);
  assert.ok(prompt.includes(profile.principalId)); assert.ok(prompt.includes(JSON.stringify(profile.scopes)));
  assert.match(prompt, /pcp_whoami/); assert.match(prompt, /pcp_list_scopes/); assert.match(prompt, /read-only search/);
  assert.match(localClientUsageGuidance("read"), /read-only/);
  assert.doesNotMatch(localClientUsageGuidance("read"), /pcp_capture/);
  assert.match(localClientUsageGuidance("contribute"), /separately authorized/);
  assert.match(localClientUsageGuidance("contribute"), /projectKey/);
  const all = localClientVerificationPrompt("pcp_one", {...profile, allScopes: true, scopes: []});
  assert.match(all, /All current and future scopes/);
  assert.doesNotMatch(all, /scope access: \[\]/);
});

test("all client guidance and interactive HTML copy have Chinese translations or are technical names", () => {
  const t = value => LOCAL_CLIENT_CONFIG_MESSAGES[value] || value;
  for (const target of LOCAL_CLIENT_TARGETS) {
    assert.match(target.docs, /^https:\/\//);
    assert.ok(target.steps.every(step => LOCAL_CLIENT_CONFIG_MESSAGES[step]), target.id);
  }
  assert.match(localClientVerificationPrompt("pcp_one", { principalId: "local", scopes: [] }, t), /预期身份/);
  assert.match(localClientUsageGuidance("contribute", t), /项目范围/);
  const html = readFileSync(new URL("../src/index.html", import.meta.url), "utf8");
  const dialog = html.slice(html.indexOf('<dialog id="local-client-dialog"'), html.indexOf('<dialog id="action-dialog"'));
  const app = readFileSync(new URL("../src/app.js", import.meta.url), "utf8");
  for (const [, key] of dialog.matchAll(/data-i18n(?:-aria-label|-placeholder|-title)?="([^"]+)"/g))
    assert.ok(LOCAL_CLIENT_CONFIG_MESSAGES[key] || app.includes(JSON.stringify(key) + ":"), key);
});
