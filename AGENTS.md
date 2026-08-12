# Global browser automation guidance

These are personal defaults for every Codex workspace. More specific project
instructions may refine them, but must preserve the separation between the
agent-browser-managed Chrome session and the disabled legacy Chrome DevTools
profile.

## Default browser workflow

- For interactive website work, authenticated browsing, web UI testing,
  dev-server verification, DOM inspection, navigation, form entry, JavaScript
  execution, screenshots, or frontend validation, use the
  `vercel:agent-browser` skill and the installed `agent-browser` CLI/MCP tools.
- When that skill applies, read its `SKILL.md` and use its core sequence:
  navigate, take an interactive semantic snapshot, interact through fresh
  element refs, and re-snapshot after page or DOM changes.
- The configured default engine is Chrome. Use plain `agent-browser` commands;
  `/home/sabino/.local/bin/agent-browser-chrome` is an equivalent compatibility
  alias. Both address the same default socket-backed Chrome singleton.
- Reuse that default session across Codex instances. Do not create extra browser
  daemons unless isolation is materially necessary. Close it when the task is
  finished so restore state is saved and resources stop; the 10-minute idle
  timeout remains a fallback.
- Use headless Chrome by default. Use `agent-browser --headed ...` only when the
  user asks to see or manually interact with the rendered browser.
- Do not substitute browser automation for ordinary public-web research when
  search or a direct fetch is sufficient.

## Frontend and visual verification

- When correctness depends on rendered pixels or browser layout, also use the
  `vercel:agent-browser-verify` skill. This includes screenshots, screenshot
  diffs, responsive layout, CSS, fonts, canvas, SVG appearance, clipping,
  overlap, spacing, color, animation, and frontend visual regressions.
- For reproducible screenshots, set an explicit viewport, wait for the relevant
  page state, capture the artifact, and inspect the resulting image. For visual
  comparisons, keep engine, viewport, device scale, color scheme, and page state
  identical between baseline and candidate.
- A normal visual workflow is:

  ```bash
  agent-browser open <url>
  agent-browser set viewport 1440 900
  agent-browser wait --load networkidle
  agent-browser snapshot -i
  agent-browser screenshot --full <artifact.png>
  agent-browser diff screenshot --baseline <baseline.png>
  agent-browser close
  ```

- The agent-browser dashboard may display the live screencast from this Chrome
  session. Treat a connected dashboard as observability, not as a replacement
  for saving and inspecting required screenshot artifacts.

## Lightpanda boundary

- Lightpanda remains installed for explicitly requested low-resource semantic
  workloads, but it is not the default and must not be used for screenshots,
  visual comparison, frontend pixel validation, or a dashboard viewport.
- Never combine a Chrome executable override with the Lightpanda engine.
  Lightpanda has no graphical renderer, and its placeholder image is not visual
  evidence.
- Browser restore files contain authentication secrets. Never print cookie
  values, place restore state in a repository, or loosen its private file
  permissions.

## Disabled legacy Chrome path

- Do not use or start the `chrome-devtools` MCP server, the
  `/home/sabino/.codex/bin/codex-chrome-devtools-mcp` wrapper, the
  `codex-chrome-browser.service` user service, or the dedicated
  `/home/sabino/.codex/chrome-devtools-profile` profile.
- This legacy Chrome path is retained only as disabled compatibility data. Do
  not attach to it, inspect it, modify it, or migrate data from it unless the
  user explicitly requests that legacy fallback for the current task.
- Authorization to use Chrome through `agent-browser` does not authorize
  re-enabling the `chrome-devtools` MCP or accessing the retained legacy profile.

--- project-doc ---

# OpenAPI FDW contributor guidance

This repository implements a native, read-only PostgreSQL FDW for JSON HTTP
APIs. The runtime is Rust with pgrx and Supabase Wrappers. The former
Hy/Python/Multicorn implementation is historical and must not be reintroduced
as an in-database dependency.

## Important paths

- `src/fdw.rs`: PostgreSQL scan/import callbacks and JSON-to-cell conversion.
- `src/http.rs`: bounded pooled HTTP client, redirects, retries, and spec fetch.
- `src/spec.rs`: OpenAPI 3.0/3.1 import and safe SQL generation.
- `src/options.rs`: validated server/table options and credential redaction.
- `control-plane/`: stateless Rust web UI/API for discovery and transactional
  configuration; it stores only redacted source definitions.
- `examples/brasilapi.openapi.yaml`: directly importable public demo contract.
- `tests/mock_api.py`: deterministic real HTTP origin for integration tests.
- `tests/sql/native_integration.sql`: end-to-end PostgreSQL assertions.
- `tests/control_plane.sh`: browser-independent control-plane integration flow.
- `Dockerfile`: per-PostgreSQL-major build and minimal runtime image.
- `Dockerfile.control`: stripped static control-plane image.

## Validation

Use `cargo fmt --all -- --check` for the quick local check. A full validation
must build the extension for the target PostgreSQL major, run an actual
PostgreSQL server, and execute `tests/sql/native_integration.sql` against the
deterministic API. Mock-only Rust/Python results are not sufficient evidence.

The CI matrix covers PostgreSQL 14 through 18. Live public API checks are
separate from deterministic merge checks because external availability is not
under this project's control.

Control-plane changes must also pass
`cargo test --locked --package openapi-fdw-control`, `node --check
control-plane/assets/app.js`, and the real PostgreSQL flow in
`tests/control_plane.sh`. If rendered behavior changes, verify it through the
configured `agent-browser` Chrome session at desktop and narrow viewports.

## Resource and safety constraints

Rust/pgrx release builds can create roughly 2 GiB of intermediates. On a
constrained workstation, use a bounded tmpfs target or let CI build the matrix.
Never broadly prune Docker images, build cache, or volumes; remove only exact
task-owned artifacts after confirming their identity.

Keep HTTPS verification enabled by default, preserve response/time/page caps,
never include credentials in errors or fixtures, and do not forward redirects
or pagination across origins without an explicit opt-in.
