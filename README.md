# Chalk

```text
  __________
    __   /
-  /__) /_ -   CHALK
  /__) /__)    Blackboard, without the friction.
 __________
```

Your courses, without the maze. Browse Blackboard, read announcements, open lecture files, and gather course context for an AI study assistant.

## Get started

**Early alpha:** [download for Linux x64](https://github.com/Zekiah-A/chalk/releases/tag/v0.1.0-alpha.1). Extract the archive and open a terminal in the extracted folder. You’ll need Brave or Chromium and a viewer for your lecture files.

```sh
./chalk login learn.youruni.ac.uk
./chalk
```

Replace `learn.youruni.ac.uk` with your university’s Blackboard address. A browser window opens: sign in as usual, including MFA. Next time, run `./chalk` from that folder. Prefer to install `chalk` as a command, or build on macOS? See [installation options](docs/usage.md#installation).

Choose a course, then a section or folder. **Type** to filter, **Enter** to open, **Esc** to go back. Read text in the terminal; select lecture files to download and open them in your usual viewer.

## Bring the context with you

To bulk download Blackboard context—for studying, an assignment, or an AI agent—run `./chalk grab` from that folder.

Choose your course, filter the resources, and tick what you want. Chalk saves the files and readable text together. Give your AI tool the export folder, starting with `CONTEXT.md`.

You can also choose **Export this section** or **Export this item** while browsing.

## Find your way

- **Using Chalk:** [sign-in, browsing, downloads and troubleshooting](docs/usage.md).
- **Agents and developers:** [MCP setup, commands and technical reference](docs/development.md).
- **Feedback:** [report a problem or suggest an improvement](https://github.com/Zekiah-A/chalk/issues).

Compatibility with your university’s specific configuration is not guaranteed. Chalk is an independent project, unaffiliated with your university or Blackboard.

Licensed under [GNU GPLv3](LICENSE). Copyright © 2026 Zekiah-A.
