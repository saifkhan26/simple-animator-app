# `krita-plugin/animator_link/__init__.py` — plugin entry point

> **Source:** [krita-plugin/animator_link/__init__.py](../../../krita-plugin/animator_link/__init__.py) ·
> **Python module:** `animator_link` · **Size:** 5 lines

## What this file is for

When Krita loads the plugin it imports the `animator_link` package, which runs this file. It does one
thing: creates the plugin object and registers it with Krita.

```python
from krita import Krita
from .extension import AnimatorLink

Krita.instance().addExtension(AnimatorLink(Krita.instance()))
```

- `from krita import Krita` — Krita's Python API (only exists inside Krita).
- `from .extension import AnimatorLink` — a *relative import* (the leading `.` means "from this
  package"), like `import { AnimatorLink } from "./extension.js"`.
- `Krita.instance()` — the running Krita application (a singleton).
- `addExtension(...)` — registers an *Extension*, Krita's kind of plugin that lives as long as Krita
  runs. Krita then calls its `setup()` (see [extension.py](extension.md)).

## How it connects

- **Uses:** [extension.py](extension.md).
- **Loaded by:** Krita, because [animator_link.desktop](../animator_link.desktop.md) names this package.

## Python concepts in this file

**Packages and `__init__.py`.** A folder containing `__init__.py` is a Python *package*; importing the
package runs this file — like a folder's `index.js`.
