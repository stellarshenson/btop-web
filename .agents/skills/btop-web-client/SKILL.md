---
name: btop-web-client
description: Reads live host telemetry (CPU, memory, network, processes, GPU) from a btop-web server or agent through the `btop-web-client` CLI of the btop-web-client Python package. Use when checking load, top processes or GPU use on a host that runs btop-web, or "read the btop-web stream".
---

# btop-web-client

Reads btop-web `/events.json`, prints frames as JSON lines. Needs network reach to server, nothing else. Commands, flags, output, errors: `btop-web-client --help`. Read first.

## Rules

- Frame holds process command lines and user names. Secrets possible. Keep needed fields, never paste whole frame into shared place
- Frame big. Filter, keep needed fields
- Follow mode never ends alone. Bound it with `head` or `timeout`
- Access key stays in file. Never print, never put on command line
- Pin refused: certificate changed or pin missing. Stop, ask user. Never pin fingerprint from error unchecked
