# Installing ls_web

## From source (recommended)

Build a release-optimized binary and install it to your Cargo bin directory:

```sh
make release      # build optimized binary
make install      # install to $HOME/.cargo/bin (or your cargo bin dir)
```

After installing, you can run `ls_web` from any directory:

```sh
ls_web --dir /path/to/serve --host 0.0.0.0 --port 7878
```

## Quick run without installing

If you just want to try it without installing, run:

```sh
make run
```

Or for a release build:

```sh
make run-release
```
