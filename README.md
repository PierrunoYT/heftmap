<p align="center"><img src="assets/logo.png" alt="Heftmap logo: a disk platter split into a colorful treemap" width="160"></p>

<h1 align="center">Heftmap</h1>

<p align="center"><strong>See what fills your disk, folder by folder.</strong></p>

<p align="center">
  <img src="https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-0078D4" alt="Platforms: Windows | macOS | Linux">
  <img src="https://img.shields.io/badge/made%20with-Rust-dea584?logo=rust" alt="Made with Rust">
</p>

<p align="center">
  <a href="#build-and-run">Get started</a> ·
  <a href="#features">Features</a> ·
  <a href="#how-sizes-are-counted">How sizes are counted</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

Heftmap is a free, open-source disk usage analyzer for Windows, Linux and macOS, in the spirit of
[TreeSize](https://www.jam-software.com/treesize_free) and written in Rust with
[egui](https://github.com/emilk/egui). It scans a folder or drive in parallel and shows where the
space went as a sortable folder tree, a treemap, a breakdown by file type and a list of the
largest files.

Heftmap is one of the open-source desktop apps from [Pierruno](https://pierruno.dev/).

## Features

- **Parallel scan.** Any folder or drive, with live progress (files, folders, bytes, current path)
  and Cancel; a cancelled rescan keeps the previous results.
- **Folder tree sorted by size.** Columns for size, allocated size on disk, a % of parent bar,
  recursive file and folder counts and the newest change anywhere below. Sort by any column, hide
  files, and navigate with the keyboard (arrows, ←/→ collapse/expand, Home/End, PageUp/PageDown).
- **Treemap.** A squarified, nested treemap of the selected folder: click selects in the tree,
  double-click zooms in, right-click or **⬆** zooms out, hovering shows path and size.
- **File types.** Bytes and file count per extension (case-insensitive) below the selected folder.
- **Largest files.** The top 250 files below the selected folder; clicking one reveals it in the tree.
- **Cloud-aware sizes.** Rankings use the space actually allocated on disk, so online-only
  OneDrive or kDrive placeholders don't look like they fill the drive.
- **Unreadable folders.** Folders that can't be read are marked ⚠ and counted in the status bar.
  On Windows, **🛡 Restart as administrator** rescans the same folder elevated.
- **Shell actions.** Open in Explorer/Finder/file manager, copy path, show a folder in the side panel.
- **Report mode.** Print a size-sorted tree to the terminal for scripts.
- **Light on memory.** OpenGL rendering and a compact tree: about 140 MB idle, 60 MB peak for a
  460k-entry scan.

## Platforms

| | Windows | Linux | macOS |
| --- | :---: | :---: | :---: |
| Scan, tree, treemap, file types, largest files | ✓ | ✓ | ✓ |
| Allocated size on disk | ✓ | ✓ | ✓ |
| Drives menu | drive letters | `/` and home | `/` and home |
| Restart as administrator | ✓ | | |
| Scan stays on one file system | | ✓ | ✓ |

## Build and run

You need a recent stable [Rust toolchain](https://rustup.rs/).

**Windows**

```powershell
cargo build --release
.\target\release\heftmap.exe
```

**Linux (Debian/Ubuntu)**

```sh
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 xdg-utils zenity
cargo build --release --locked
./target/release/heftmap "$HOME"
```

Run the GUI from a Wayland or X11 desktop session. `libxkbcommon-x11-0` is required
on X11; without it the app can build but panics at launch. `zenity` lets **Browse…**
work when no desktop file chooser portal is available, and `xdg-utils` provides
`xdg-open` for the file manager action. Report mode does not need a desktop session:

```sh
./target/release/heftmap --report "$HOME" --depth 2 --top 10
```

**macOS**

In addition to Rust, you need the Xcode Command Line Tools or full Xcode. If neither is
installed, run `xcode-select --install` first. Then, from the repository:

```sh
cargo build --release --locked
./target/release/heftmap
```

The release build, native folder picker, and report mode were verified on Apple Silicon
(`arm64`) with macOS 27.2 and Rust 1.99.1. Intel Macs and older macOS versions were not tested.

## Usage

```sh
heftmap                                            # open the GUI
heftmap D:\                                        # open the GUI and scan D:\
heftmap --report D:\Projects --depth 3 --top 15    # print a size-sorted tree
heftmap --help
```

`--depth` sets the levels printed below the path (default 2), `--top` the largest entries per folder
(default 10). Each line shows size, size on disk and % of parent; the rest of a folder is summed in
a `… N more` line:

```text
D:\Github\Pierruno Apps\Heftmap\src  84.4 KB (100.0 KB on disk)  (9 files, 0 folders)  scanned in 2.25ms
      size    on disk
   33.5 KB    36.0 KB   36.0%  app.rs
    9.9 KB    12.0 KB   12.0%  scan.rs
   11.7 KB    12.0 KB   12.0%  tree.rs
    8.7 KB    12.0 KB   12.0%  treemap.rs
   20.6 KB    28.0 KB   28.0%  … 5 more
```

Release builds on Windows have no console window; report mode attaches to the terminal it was
started from.

## How sizes are counted

- **Size** is the logical file length. **Allocated** is the space actually used on disk:
  cluster-rounded, smaller for compressed or sparse files, and 0 for online-only cloud placeholders.
  It is read from the directory listing itself (`FILE_FULL_DIR_INFO` on Windows, `st_blocks` on
  Unix), so it costs no extra time.
- Percentages, the default sort, the treemap, file types and largest files use Allocated.
- 1 KB = 1024 bytes.
- Symlinks and junctions are not followed, so nothing is counted twice through them. Hard links
  count once per link.
- On Linux and macOS the scan stays on the starting folder's file system (no `/proc`, network
  mounts, …).
- Elevated scans on Windows enable the backup privilege, so folders that deny even administrators
  (such as `System Volume Information`) are listed too.

## Development

```sh
cargo clippy --all-targets -- -D warnings
cargo test
```

The window and `.exe` icons are generated from `assets/logo.png`: `assets/icon.png` (256 px) is
embedded as the window icon, and `build.rs` embeds `assets/icon.ico` into the Windows executable.
