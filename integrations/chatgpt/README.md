# ChatGPT local integration

This integration lets ChatGPT Developer Mode call the local PCP stdio MCP server through OpenAI Secure MCP Tunnel. The tunnel process makes an outbound HTTPS connection; PCP Runtime, its Unix sockets, Store, and enrollment credential remain local.

It is intended for private development and personal use. It is not the public deployment path for a ChatGPT app or a Codex plugin.

## 1. Install PCP

Build and install Runtime, Console, and `pcp-mcp`:

```bash
sh scripts/install-macos.sh
```

The installer places the ChatGPT launcher at:

```text
~/Library/Application Support/PCP/bin/pcp-chatgpt-mcp
```

## 2. Enroll a separate ChatGPT Principal

Find the current PCP Infra Discovery registration manifest, then begin an enrollment. Do not reuse the Codex credential or Principal. The launcher fixes the requested policy to `contribute` on `user:self` plus read-only access to all current Scopes.

```bash
PCP_HOME="$HOME/Library/Application Support/PCP"
"$PCP_HOME/bin/pcp-chatgpt-mcp" enroll begin \
  "/absolute/path/to/current/pcp--idn_....json"
```

Approve `ChatGPT` in PCP Console, then complete the state file:

```bash
PCP_HOME="$HOME/Library/Application Support/PCP"
"$PCP_HOME/bin/pcp-chatgpt-mcp" enroll status
```

The requested policy contributes only to `user:self` and reads all current Scopes. Runtime resolves `user:self` to this Store's user Scope. Reopen the MCP session after a new Scope is created if it should become readable.

## 3. Configure Secure MCP Tunnel

Create a tunnel in the OpenAI Platform, install the current `tunnel-client`, and initialize a local stdio profile with the launcher as its MCP command:

```bash
tunnel-client init \
  --sample sample_mcp_stdio_local \
  --profile pcp-chatgpt \
  --tunnel-id <tunnel_id> \
  --mcp-command "$HOME/Library/Application Support/PCP/bin/pcp-chatgpt-mcp"

tunnel-client doctor --profile pcp-chatgpt
tunnel-client run --profile pcp-chatgpt
```

Keep the tunnel process running. In ChatGPT Developer Mode, create an app using **Tunnel** and select the same tunnel ID. Review the discovered tools and their action controls before enabling writes.

## Boundaries

- `pcp_search_pages`, `pcp_semantic_search`, `pcp_read_pages`, and graph inspection are read-only.
- `pcp_capture` and `pcp_submit_feedback` are declared as write actions. ChatGPT captures use Page kind `chatgpt_capture` and facet `captureSurface: chatgpt`.
- The MCP server instructions apply the same high capture threshold as the Codex plugin, but ChatGPT does not load the Codex Skill.
- The tunnel does not grant PCP access. Runtime still requires the approved `chatgpt:pcp` enrollment on every MCP process start.
- Do not put the PCP credential, Store, Runtime socket, tunnel runtime API key, or tunnel configuration in this repository.

Official setup references: [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels) and [connect and test in ChatGPT](https://developers.openai.com/plugins/deploy/connect-chatgpt).
