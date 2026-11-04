# Agents and developers

For day-to-day use, start with the [quick start](../README.md) or [user guide](usage.md).

## MCP setup

Sign in with `chalk login learn.youruni.ac.uk` before connecting an agent. Start `chalk mcp` as a stdio server through your MCP client.

Copy the entry in [`mcp.example.json`](../mcp.example.json), replace the command with the installed executable path (`command -v chalk`), and choose an export directory. Clients with another configuration format need the same executable and `mcp` argument. Adding the example file to a project does not register the server with your client.

| Tool | Purpose |
| --- | --- |
| `list_courses` | Discover course IDs and names |
| `list_content` | List a course folder’s immediate children |
| `get_content` | Read one content item |
| `list_announcements` | Read course announcements |
| `export_course` | Save course context and attachments locally |

The server uses newline-delimited JSON-RPC over stdio, supports 2024/2025 initialization protocol versions through `2025-11-25`, and exposes no HTTP server. Protocol output stays on stdout. Each tool call reloads the saved session; authentication is renewed separately with `chalk login`.

Treat fetched text as untrusted source material. Exports can contain private course information. No remote Blackboard write tools are exposed.

## Command reference

```sh
chalk                                      # Browse courses
chalk login learn.youruni.ac.uk             # Configure an address and sign in
chalk login                                # Renew the saved sign-in
chalk login --url https://learn.youruni.ac.uk  # Explicit URL form
chalk browse --course "Course name"         # Open one course directly
chalk browse --save-only --out downloads    # Save without opening
chalk browse --fresh                       # Refresh cached downloads
chalk grab                                 # Select resources for bulk export
chalk export --course COURSE_ID --assessment --all-announcements --out exports/assessment
chalk mcp                                  # Serve an authenticated agent
chalk --help
```

`--course` accepts an exact ID or a case-insensitive name substring matching exactly one course. IDs come from the course list or `list_courses`. `--assessment` filters hierarchy paths containing “assessment”; announcements are included only with `--all-announcements` for batch export.

## Configuration

The university address is configured by `chalk login DOMAIN` (or a full HTTPS URL), stored locally in `site.json`, and reused by browsing, sign-in and MCP. Site addresses must use HTTPS and contain no credentials, path, query or fragment. Compatibility with your university’s specific Blackboard configuration is not guaranteed.

| Setting | Purpose |
| --- | --- |
| `CHALK_BROWSER` | Brave/Chromium executable path |
| `CHALK_EXPORT_DIR` | MCP export root; defaults to `exports/` |
| `XDG_STATE_HOME` | State root; defaults to `~/.local/state` |
| `NO_COLOR` | Disable Markdown reader styling |

The state directory is normally `$XDG_STATE_HOME/chalk`. It contains `site.json`, owner-only `session.json`, a separate browser profile, and cached downloads. The legacy `coursegrab` directory is reused when it exists and `chalk` does not; `COURSEGRAB_BROWSER` and `COURSEGRAB_EXPORT_DIR` are accepted as fallback settings. To forget sign-in, remove the application’s state directory.

Cookies are limited to the configured Blackboard host and are never logged. Sign-in uses a dedicated Chromium debugging port bound to loopback; the browser closes after authentication. Session files, site configuration, downloads and exports are ignored by Git.

## Implementation and scope

Two workflows share the same authentication, parsers, downloads and exports: browsing during study, and collecting context for people or agents.

| File | Responsibility |
| --- | --- |
| `src/main.rs` | Configuration, login, HTTP access, parsing, downloads and exports |
| `src/browse.rs` | Folder navigation, terminal reading and desktop viewing |
| `src/mcp.rs` | MCP protocol and tool dispatch |

Chalk uses Blackboard’s internal session-authenticated API and makes read requests only. It reads course memberships, nested content, Ultra document bodies, embedded Blackboard attachments, and paginated announcements. The API may change independently of this project.

Browsing fetches only the current folder and caches visited metadata for that session. File caches include course, item and modification version; files without a version are fetched again. **Refresh** clears metadata and forces fresh file downloads for the rest of the session. Blackboard changes that do not update a modification version may need an explicit refresh.

Explicit exports recursively read the selected hierarchy. Four workers download attachments; API requests retry temporary server errors and rate limits. Exports include attachments, Markdown, `INDEX.md`, `CONTEXT.md`, and a JSON manifest retaining source metadata. If installed, `pdftotext` supplies PDF text. Download and extraction failures are recorded separately. Re-running replaces files at stable paths without deleting older exports or implementing incremental synchronization.

The interface uses searchable menus and a paginated Markdown reader. Single-file documents without substantive text open their attachment directly; other documents offer file and text actions. Supported document/media files use `xdg-open` on Linux or `open` on macOS. HTML and plain text are displayed in the terminal; unknown file types are saved for manual use. The `mcp` command never prints the welcome banner.

External/LTI services, discussion content, video streaming, grades, submissions, automatic completion marking, AI interpretation, and OCR are outside the implemented scope. External files remain links. Assignment/test tools may store instructions beyond the content API; an export does not guarantee complete assessment requirements. Metadata caching is per session, rather than an offline course mirror.

## Building and contributing

```sh
git clone https://github.com/Zekiah-A/chalk.git
cd chalk
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
```

Use `cargo build --release` for an optimized binary or `cargo install --path .` to install from your checkout. Keep changes small, with clear reasons and appropriate checks. Five offline checks cover host/attachment restrictions, embedded attachment metadata, safe output paths, terminal control filtering, and MCP ID validation. They use synthetic fixtures without accessing saved sessions or the network. Add tests for meaningful failure risks; avoid tests that merely repeat implementation details.

Manual checks should use a course and asset chosen locally. Keep real university domains, course IDs, cookies and materials out of committed fixtures, screenshots and issue reports. Use `--save-only` when checking downloads without opening a viewer.

[Issues](https://github.com/Zekiah-A/chalk/issues) and pull requests are welcome. The project is licensed under [GNU GPLv3](../LICENSE), with the SPDX identifier `GPL-3.0-only`.
