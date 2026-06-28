# Keybindings

## Default Shortcuts

| Action | Linux/Win | macOS |
|--------|-----------|-------|
| Command palette | `Ctrl+Shift+P` | `Cmd+Shift+P` |
| File finder | `Ctrl+P` | `Cmd+P` |
| Open file | `Ctrl+O` | `Cmd+O` |
| Save | `Ctrl+S` | `Cmd+S` |
| Save all | `Ctrl+Shift+S` | `Cmd+Shift+S` |
| Close tab | `Ctrl+W` | `Cmd+W` |
| Quit | `Ctrl+Q` | `Cmd+Q` |

### Navigation

| Action | Linux/Win | macOS |
|--------|-----------|-------|
| Go to line | `Ctrl+G` | `Cmd+G` |
| Go to definition | `F12` | `F12` |
| Go back | `Alt+Left` | `Ctrl+-` |
| Go forward | `Alt+Right` | `Ctrl+Shift+-` |
| Go to file | `Ctrl+P` | `Cmd+P` |

### Editing

| Action | Linux/Win | macOS |
|--------|-----------|-------|
| Undo | `Ctrl+Z` | `Cmd+Z` |
| Redo | `Ctrl+Shift+Z` | `Cmd+Shift+Z` |
| Cut | `Ctrl+X` | `Cmd+X` |
| Copy | `Ctrl+C` | `Cmd+C` |
| Paste | `Ctrl+V` | `Cmd+V` |
| Select all | `Ctrl+A` | `Cmd+A` |
| Duplicate line | `Ctrl+Shift+D` | `Cmd+Shift+D` |
| Delete line | `Ctrl+Shift+K` | `Cmd+Shift+K` |

### Search

| Action | Linux/Win | macOS |
|--------|-----------|-------|
| Find | `Ctrl+F` | `Cmd+F` |
| Find in project | `Ctrl+Shift+F` | `Cmd+Shift+F` |
| Find next | `F3` / `Ctrl+G` | `Cmd+G` |
| Find previous | `Shift+F3` / `Ctrl+Shift+G` | `Cmd+Shift+G` |
| Replace | `Ctrl+H` | `Ctrl+Cmd+F` |

### Panels

| Action | Linux/Win | macOS |
|--------|-----------|-------|
| Toggle terminal | `Ctrl+\`` | `Cmd+\`` |
| Toggle file explorer | `Ctrl+Shift+E` | `Cmd+Shift+E` |
| Toggle git panel | `Ctrl+Shift+G` | `Cmd+Shift+G` |
| Toggle AI panel | `Ctrl+Shift+A` | `Cmd+Shift+A` |
| Toggle outline | `Ctrl+Shift+O` | `Cmd+Shift+O` |

### AI Agent

| Action | Linux/Win | macOS |
|--------|-----------|-------|
| Open AI panel | `Ctrl+Shift+A` | `Cmd+Shift+A` |
| Inline assistant | `Ctrl+I` | `Cmd+I` |
| Accept suggestion | `Tab` | `Tab` |
| Reject suggestion | `Escape` | `Escape` |

## Custom Keybindings

Create `~/.config/tau/keymap.json`:

```json
[
  {
    "context": "Editor",
    "bindings": {
      "ctrl-shift-alt-l": "editor::Format",
      "ctrl-d": "editor::DuplicateLine",
      "alt-up": "editor::MoveLineUp",
      "alt-down": "editor::MoveLineDown"
    }
  }
]
```

Full default keymaps: [`assets/keymaps/`](../../assets/keymaps/)
