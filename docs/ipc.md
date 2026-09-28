# IPC protocol

Socket: `$XDG_RUNTIME_DIR/edex-de/ipc.sock` (mode 0600). One JSON object per line, one reply per request.

Request objects carry `"cmd"` (kebab-case) plus fields:

| cmd | fields | effect |
|---|---|---|
| `ping` | | `{"ok":true,"data":{"version":…,"uptime_secs":…}}` |
| `toggle` / `show` / `hide` | `target`: `launcher`, `settings`, `privacy`, `notifications`, `power` | overlay control |
| `focus` | `target`: `terminal` or `filesystem` | shell panel focus |
| `audio` | `op`: `volume` (+`delta` or `set`), `mute`, `mic-mute` | wpctl + OSD |
| `brightness` | `delta` | brightnessctl + OSD |
| `theme` | `name` | switch and save the theme |
| `reload` | | re-read config, regenerate the Hyprland export |
| `state` | | JSON snapshot (outputs, frames, overlay, terminal, Hyprland, status) |
| `screenshot-scene` | | rect count and visible strings of the primary canvas (for tests) |
| `notify` | `summary`, `body` | local notification |
| `action` | `name`: `install`, `lock`, `new-tab`, `keyboard` | shell actions |
| `quit` | | exit the shell (the user service restarts it) |

Replies: `{"ok":true}`, `{"ok":true,"data":…}` or `{"ok":false,"error":"…"}`.

The `edex-de ipc` CLI turns arguments into requests: `edex-de ipc audio volume +5`,
`edex-de ipc toggle launcher`, `edex-de ipc state`.
