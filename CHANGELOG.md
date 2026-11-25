# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-11-17

### Added

- Heftmap: a TreeSize-style disk usage analyzer (egui) for Windows, Linux and macOS.
- App logo (a disk platter split into a colorful treemap), shown in the window and taskbar and
  embedded in the Windows `.exe` by `build.rs`.
- Parallel directory scanner (rayon) with live progress (files, folders, bytes, current path)
  and Cancel; a cancelled rescan keeps the previous results.
- Folder tree with size, **Allocated** (size on disk), a % of parent bar, recursive file and
  folder counts and the newest change below each folder. Sortable by every column, virtualized
  rows, optional hiding of files, keyboard navigation (arrows, Home/End, PageUp/PageDown).
- Side panel scoped to the selected folder:
  - Squarified, nested treemap: click selects, double-click zooms in, right-click or **⬆** zooms
    out, hovering shows path and size.
  - File types: bytes and file count per extension, case-insensitive.
  - Largest files: the top 250; clicking one reveals it in the tree.
- Disk usage is ranked by allocated size: % of parent, the default sort, treemap areas, file
  types, largest files, the side panel total and `--report` percentages. Allocated size is read
  from the directory listing itself (`FILE_FULL_DIR_INFO` on Windows, `st_blocks` on Unix), so
  scans are no slower, and online-only cloud placeholders (OneDrive, kDrive) count as 0 instead
  of appearing to fill the disk.
- Context menu: open in Explorer, Finder or the file manager (`xdg-open`), copy path, show in
  side panel.
- Toolbar with path entry, folder picker, Drives menu (drive letters on Windows, `/` and the home
  folder elsewhere) and Scan/Rescan.
- Status bar with totals (size and size on disk), scan time and the count of unreadable entries;
  unreadable folders are marked ⚠.
- **🛡 Restart as administrator** (Windows) next to the count of unreadable folders: relaunches
  through UAC and rescans the same folder. Elevated scans enable the backup privilege
  (`SeBackupPrivilege`), so folders whose ACLs exclude administrators are listed as well.
- Report mode: `heftmap --report PATH [--depth N] [--top N]` prints a size-sorted tree with
  size, size on disk and % of parent; `--help` prints usage. A bare drive such as `C:` means its
  root.
- Release builds on Windows run without a console window; report mode attaches to the calling
  console.
- Low memory use: OpenGL (glow) rendering, about 140 MB idle; fixed-size tree nodes, one shared
  name buffer and child index, and no per-file allocation in the scanner (60 MB peak for a
  460k-entry scan).
- Symlinks and junctions are not followed and hard links count once per link. On Unix, scans
  stay on the starting folder's file system.
- README with logo, badges, quick links, a platform table, a report mode example and how sizes
  are counted. The GitHub repository has a description and topics.
