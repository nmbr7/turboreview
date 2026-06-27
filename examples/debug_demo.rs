//! A tiny program for exercising turboreview's in-TUI debugger.
//!
//! Build it as a Cargo example (produces `target/debug/examples/debug_demo`):
//!
//! ```sh
//! cargo build --example debug_demo
//! ```
//!
//! Then point the `debug` block in `.turboreview/config.json` at that binary
//! (see the project README / docs). Good breakpoint spots are marked below.

/// Sum 1..=n the slow way so there's a loop with a changing local to inspect.
fn accumulate(n: u32) -> u64 {
    let mut total: u64 = 0;
    for i in 1..=n {
        // BREAKPOINT: set one here and watch `total` and `i` update each step.
        total += i as u64;
    }
    total
}

/// A second frame so the call stack has depth to walk in the Debug panel.
fn describe(label: &str, value: u64) -> String {
    let formatted = format!("{label} = {value}");
    // BREAKPOINT: stop here to see `label`, `value`, and `formatted` as locals,
    // with `accumulate` / `main` below this frame in the call stack.
    formatted
}

fn main() {
    let n: u32 = 10;
    let sum = accumulate(n);
    let report = describe("sum(1..=10)", sum);
    println!("{report}");
    // BREAKPOINT: a final stop with `n`, `sum`, and `report` in scope.
    std::process::exit(0);
}
