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
    "source_map": [],
    "remote": { "host": "", "port": 0, "attach_commands": [] }
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
- `remote` — remote-attach target: `host`/`port` build an lldb-dap
  `gdb-remote host:port`, or set `attach_commands` to raw adapter commands.

## 4. Debug it

Run turboreview in this repo, open `examples/debug_demo.rs` in the diff, then:

1. `b` on a marked `// BREAKPOINT` line — a `●` appears in the gutter.
2. `D` opens the **launch picker** — choose *worktree* (or *commit* in the
   Commits view, *process*, *remote*); it builds (if configured) and starts.
3. The right pane's **Debug** tab shows `⏸ stopped`, the call stack, and the top
   frame's locals; `▶` marks the stopped line. `[`/`]` switch the right tab,
   `t` switches Vars / Breakpoints.
4. `Tab` to focus the Debug pane, then step: `c` continue, `n` next, `i` step-in,
   `o` step-out. `Enter` expands a frame or a structured variable.
5. Attach a snapshot: while stopped, press `c` to comment, then `Ctrl-D` to
   attach the current call stack + locals to the comment.
6. `X` ends the session.

## Full config reference

Beyond `debug`, `.turboreview/config.json` also holds:

```jsonc
{
  // test coverage (% to toggle, M to run)
  "coverage_file": "coverage/lcov.info",
  "coverage_command": "cargo llvm-cov --lcov --output-path coverage/lcov.info",

  // macro-expanded view (e). Several entries -> a selection popup; {file} and
  // {module} are substituted (module derived from the src-relative path).
  "expand_commands": [
    { "name": "lib",     "command": "cargo expand --lib {module}" },
    { "name": "example", "command": "cargo expand --example $(basename {file} .rs)" }
  ]
}
```

See the README's *Test coverage* and *Macro-expanded view* sections for details.
