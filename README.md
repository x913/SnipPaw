# SnipPaw

Desktop app for finding and precisely clipping moments from streams.

The first stage supports local projects:

- creating a named project;
- choosing a source video through the system dialog;
- keeping a reference to the source file without copying it;
- displaying created projects;
- previewing a video and precisely choosing the start and end of a future clip;
- exporting the selected segment as MP4 (H.264/AAC).

## Development

```bash
npm install
npm run tauri dev
```

For a production build:

```bash
npm run tauri build
```

Projects are stored in the app data directory. The source video stays in its original location; each project records its path, name, and size. If the file is moved, a future version of the interface can ask for its new location.

Files for newly created projects and their clips are stored in `~/Movies/SnipPaw/projects/<project-name>-<id>/`. The app uses an installed FFmpeg for export; on macOS it automatically looks for Homebrew's path (`/opt/homebrew/bin/ffmpeg`) and the standard system path.

