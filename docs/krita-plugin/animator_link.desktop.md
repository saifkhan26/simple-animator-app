# `krita-plugin/animator_link.desktop` — the plugin's manifest for Krita

> **Source:** [krita-plugin/animator_link.desktop](../../krita-plugin/animator_link.desktop) · **Size:** 8 lines

## What this file is for

Krita finds Python plugins by scanning its `pykrita` folder for `.desktop` files — small INI-style
manifests, a bit like a `package.json` for a Krita plugin. This one tells Krita:

| Line | Meaning |
|---|---|
| `Type=Service`, `ServiceTypes=Krita/PythonPlugin` | "this is a Krita Python plugin" |
| `X-KDE-Library=animator_link` | the Python package to import: the folder [`animator_link/`](animator_link/__init__.md) |
| `X-Python-2-Compatible=false` | Python 3 only |
| `X-Animator-Version=1` | **this app's own key**: the plugin version, read by `installed_version` in [src/krita_helper.rs](../src/krita_helper.md) to offer a reinstall when the app carries a newer plugin |
| `Name`, `Comment` | what Krita's Python Plugin Manager shows |

## How it connects

- Embedded into the app binary with `include_str!` and written out by **File → Install Krita helper**
  ([src/krita_helper.rs](../src/krita_helper.md) `install_into`).
- After installing, the user enables "Animator Link" in Krita (Settings → Configure Krita → Python Plugin
  Manager) and restarts Krita.
