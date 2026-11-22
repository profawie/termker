# termker icon

`termker.svg` is the source drawing, made for this project and under the
same license as the rest of it (MIT OR Apache-2.0). The other files are
rendered from it:

| File | Used for |
| --- | --- |
| `termker.ico` | Windows program icon (16 to 256 px), embedded by `build.rs` |
| `termker.icns` | macOS app icon, copied into the bundle by `build.sh` |
| `termker-256.png` | Window and taskbar icon, and the logo inside the app |
| `termker-macos-512.png` | macOS Dock icon while running (drawn at 80% with a clear margin, as macOS icons are) |

After changing the drawing, render it again at each size and rebuild the
`.ico` and `.icns` from those renders, so small sizes stay sharp.
