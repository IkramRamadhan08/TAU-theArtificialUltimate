# Configuration

TAU is configured via JSON files in `~/.config/tau/`:

| File | Purpose |
|------|---------|
| `settings.json` | Editor settings |
| `keymap.json` | Custom keybindings |
| `themes/` | Custom themes |

## Settings

Example `~/.config/tau/settings.json`:

```json
{
  "theme": "Ayu Dark",
  "font_family": "JetBrains Mono",
  "font_size": 14,
  "tab_size": 4,
  "vim_mode": true,
  "telemetry": false,
  "hardware_acceleration": true,
  "autosave": "on_focus_change",
  "relative_line_numbers": true
}
```

## Themes

Built-in themes: Ayu Dark/Light/Mirage, Gruvbox, One Dark/Light.

```json
{ "theme": "Gruvbox" }
```

## Language-Specific Settings

```json
{
  "languages": {
    "Rust": {
      "tab_size": 4,
      "formatter": "rustfmt"
    },
    "Python": {
      "tab_size": 4,
      "formatter": "ruff"
    }
  }
}
```

## Editor Settings Reference

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `tab_size` | int | `4` | Spaces per tab |
| `font_size` | int | `14` | Editor font size |
| `font_family` | string | `"Zed Plex Mono"` | Editor font |
| `vim_mode` | bool | `false` | Enable vim emulation |
| `relative_line_numbers` | bool | `false` | Show relative line numbers |
| `autosave` | string | `"off"` | `off`, `on_focus_change`, `after_delay` |
| `telemetry` | bool | `true` | Send anonymized usage data |
| `hardware_acceleration` | bool | `true` | GPU-accelerated rendering |
