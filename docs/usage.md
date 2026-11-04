# Using Chalk

[← Back to the quick start](../README.md)

## Sign in

Find your university’s Blackboard address in your browser, then use its domain:

```sh
chalk login learn.youruni.ac.uk
```

Replace the example with your own address. Full HTTPS addresses also work. Sign in normally in the browser window, including MFA. The window closes when Chalk is ready, and your sign-in is saved on this device.

Next time, run `chalk`. If your session expires, run `chalk login` to sign in again. To change university addresses, include the new address in that command.

## Find what you need

Run `chalk`, choose a course, and navigate its sections and folders.

| Key | Action |
| --- | --- |
| Type | Filter the menu |
| ↑ / ↓ | Move between items |
| Enter | Open the selected item |
| Esc | Go back |

Announcements and text are readable in the terminal. Use **Next page**, **Previous page**, and **Back** to move through longer documents. Select a lecture file to download it and open it in your usual PDF, slides, or document viewer. If an item has several files, choose the one you need.

Already downloaded files are reused. Choose **Refresh** if something has changed. The welcome logo disappears after your first menu selection.

To download without opening files:

```sh
chalk browse --save-only
```

## Bulk download with chalk grab

To collect Blackboard context for an assignment or an AI agent, use `chalk grab`. Choose a course, select content, announcements, or both, then filter by title, folder path, or file extension. Leave the filter blank to see everything.

The checkbox menu starts with every matching item selected:

- **Space:** select or deselect an item.
- **a:** select or deselect everything.
- **Enter:** export your selection.

Selecting a folder or document includes its contents. To export a single section or item instead, choose its **Export** action while browsing.

The export folder contains your files and their readable text. Start with `CONTEXT.md` for a combined view, or `INDEX.md` to find individual resources. For an AI agent, share the folder so it can consult the original files too. Installing Poppler’s `pdftotext` tool adds extracted PDF text to exports.

## Where things are saved

Individual downloads normally live in `~/.local/state/chalk/downloads`. Export folders are saved under `exports/` in the directory where you started Chalk; the completed export prints its location.

To choose a different download folder:

```sh
chalk browse --out ~/Downloads/chalk
```

Keep your exports private: they can include course materials, announcements, and identifying source links. Your saved sign-in is stored separately on your device.

## If something goes wrong

| Problem | Try this |
| --- | --- |
| `chalk` isn’t found after installation | Reopen your terminal. Check that Rust’s installation completed. |
| Sign-in expired or access denied | Run `chalk login` and complete sign-in again. |
| Browser not found | Install Brave or Chromium. Custom browser paths are covered in the [technical reference](development.md#configuration). |
| A file won’t open | Check the saved path and open it yourself, or install a suitable viewer. A graphical desktop is needed for automatic opening. |
| A file seems out of date | Choose **Refresh**, or start with `chalk browse --fresh`. |
| Your university’s setup doesn’t work | [Report the problem](https://github.com/Zekiah-A/chalk/issues) with the command and error message. Remove personal details, university addresses, cookies, and course materials before posting. |

For submission instructions and deadlines, check the assignment’s submission page too. Some instructions live in tools that Chalk cannot read.
