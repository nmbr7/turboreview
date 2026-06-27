# Debugging the example program

turboreview ships a tiny debuggable program at `examples/debug_demo.rs` for
trying the in-TUI debugger.

## 1. Build the example

```sh
cargo build --example debug_demo
# → target/debug/examples/debug_demo
```

## 2. Install a DAP adapter

Any Debug Adapter Protocol adapter works. On macOS, `lldb-dap` ships with Xcode:

```sh
xcrun -f lldb-dap
# /Applications/Xcode.app/Contents/Developer/usr/bin/lldb-dap
```

Linux: install `lldb-dap` (LLVM) or `codelldb`.

## 3. Configure the debug target

turboreview reads `<repo>/.turboreview/config.json` (gitignored — local only).
Add a `debug` block. Example for this repo + the demo program:

```json
{
  "theme": "dark",
  "debug": {
    "adapter": {
      "command": "/Applications/Xcode.app/Contents/Developer/usr/bin/lldb-dap",
      "args": []
    },
    "build": "cargo build --example debug_demo",
    "program": "target/debug/examples/debug_demo",
    "args": [],
    "cwd": ".",
    "source_map": []
  }
}
```

Fields:
- `adapter.command` / `args` — the DAP adapter to spawn (use a full path if it's
  not on `PATH`, as with Xcode's `lldb-dap`).
- `build` — shell command run before launch (skipped if empty).
- `program` — built binary, relative to the source root.
- `args` / `cwd` — debuggee arguments and working directory.
- `source_map` — `[[from, to]]` path remaps (for old-commit / remote debugging).

## 4. Debug it

Run turboreview in this repo, open `examples/debug_demo.rs` in the diff, then:

1. `b` on a marked `// BREAKPOINT` line — a `●` appears in the gutter.
2. `D` — builds the example and launches the adapter.
3. The right **Debug** panel shows `⏸ stopped`, the call stack, and locals;
   `▶` marks the stopped line in the diff.
4. `Tab` to focus the Debug panel, then step: `c` continue, `n` next,
   `i` step-in, `o` step-out.
5. `S` attaches the current stack as a snapshot on a comment at the stopped line.
6. `X` ends the session.
