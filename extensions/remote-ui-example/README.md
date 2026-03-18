# Remote UI Example

This example now uses the managed extension sidecar contract instead of the
older one-shot `process:exec` helper. The wasm extension renders host-owned
titlebar, footer, and panel views, while `sidecar.js` handles:

- ChatGPT Codex OAuth through the same PKCE flow used by the referenced CLI
- refresh-token persistence in the extension work directory
- periodic usage reads through `https://chatgpt.com/backend-api/wham/usage`
- newline-delimited stdio JSON-RPC back to the wasm guest

Currently exercised host surfaces:

- panel
- `titlebar_widgets`
- `footer_widgets`
- command palette
- editor context menu
- project panel context menu
- panel overflow
- item tab context menu

Current remote view elements exercised by this example:

- `row`
- `column`
- `text`
- `icon`
- `button`
- `badge`
- `progress_bar`
- `divider`
- `spacer`
- `scroll_view`

The primary widget is a titlebar cluster with:

- the OpenAI icon
- a compact usage badge
- a whole-widget click target that opens the panel

The footer mounts in the new center footer zone, and the panel shows the
current account, plan, status text, and both usage windows in more detail.

Implementation notes:

- `sidecar.js` stores the refresh token in `codex-chatgpt-auth.json` under the
  extension work directory, because sidecars run with their current directory
  set to the writeable extension work dir
- relative sidecar assets are copied during packaging and remote sync, so
  `node ./sidecar.js` survives dev installs and remote mirroring
- titlebar and footer widgets opt into bounded host-driven refresh with
  `refresh_interval_seconds = 60`
- the example manifest now uses host slot sizing: titlebar `size = "m"` and
  footer `zone = "center", size = "l"`
- the runtime now validates slot content budgets, so oversized widget trees are
  rejected before mount instead of rendering unpredictably

This example intentionally targets the private ChatGPT Codex usage flow the
thread requested. It is an example integration, not a public API guarantee.

Run the full demo with:

```bash
/Users/nest/.codex/worktrees/7016/neo-zed/script/run-remote-ui-example
```
