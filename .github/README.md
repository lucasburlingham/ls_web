# ls_web

Like the ubiquitous `ls` command, `ls_web` lists the files and directories in the current directory (or one you choose), and serves them over HTTP so you can browse and download them from a browser.

## 🚀 Quick start

```sh
cargo run --release -- --host 0.0.0.0 --port 7878
```

Then open **http://0.0.0.0:7878** in a browser (or replace `0.0.0.0` with your host's IP if you want to access it from another machine).

## ⚙️ Command line flags

| Flag | Description |
|------|-------------|
| `--host`, `-h` | Host to bind to (default: `0.0.0.0`) |
| `--port`, `-p` | Port to listen on (default: `7878`) |
| `--dir`, `-d` | Directory to serve (default: current working directory) |
| `--sort-dirs-first` | When listing, show directories before files |

Example:

```sh
cargo run --release -- --dir /tmp --port 8080 --sort-dirs-first
```

## 📦 Downloading files and folders

- Clicking a file will download it directly.
- Right‑click a folder to download it as an archive:
  - **ZIP** (always supported)
  - **tar.gz** (always supported)
  - **7z** (requires the `7z` executable to be installed and on your `PATH`)

<div style="border-left: 4px solid #2a9df4; padding: 12px 16px; border-radius: 6px;">

**Note:** Downloads are streamed from disk rather than buffered entirely in RAM, so large files and archives won't exhaust your system memory.

If `7z` is not available, selecting "Download 7z" will return an error.

</div>

## 📄 License

Please review the [LICENSE.md](.github/LICENSE.md) file before using this software. The license is intentional and non-standard.
