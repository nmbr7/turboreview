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

use std::collections::HashMap;

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
    let kind = classify(value);
    let formatted = format!("{label} = {value} ({kind})");
    // BREAKPOINT: stop here to see `label`, `value`, `kind`, and `formatted` as
    // locals, with `accumulate` / `main` below this frame in the call stack.
    formatted
}

/// Classify a value as even/odd — a small extra frame to step into.
fn classify(value: u64) -> &'static str {
    if value % 2 == 0 {
        "even"
    } else {
        "odd"
    }
}

/// A struct so the debugger can show fields when this value is expanded.
/// (Fields are read by the debugger, not the program — silence dead_code.)
#[derive(Debug)]
#[allow(dead_code)]
struct Point {
    x: i32,
    y: i32,
    label: String,
}

/// Build a few heap-allocated values to inspect in the Debug panel:
/// - `text` (String): the contents print directly in the value column.
/// - `numbers` (Vec<u64>): a summary inline; expand (Enter) to see [0], [1]…
/// - `point` (struct): expand to see x / y / label fields.
/// - `index` (HashMap): expand to see entries.
fn inspect_heap(sum: u64) -> usize {
    let text: String = format!("sum is {sum}");
    let numbers: Vec<u64> = (1..=sum).collect();
    let point = Point {
        x: 3,
        y: 7,
        label: String::from("origin-ish"),
    };
    let mut index: HashMap<String, u64> = HashMap::new();
    index.insert("answer".to_string(), 42);
    index.insert("sum".to_string(), sum);

    // BREAKPOINT: the heart of the demo. In the Debug panel:
    //   text   prints "sum is 55" directly (no expand needed)
    //   numbers shows a Vec summary — press Enter to expand [0], [1], …
    //   point  — expand for x / y / label (label is itself a String)
    //   index  — expand for the map entries
    numbers.len() + index.len() + point.x as usize + text.len()
}

fn main() {
    let n: u32 = 10;
    let sum = accumulate(n);
    let report = describe("sum(1..=10)", sum);
    let total = inspect_heap(sum);
    println!("{report}; total={total}");
    // BREAKPOINT: a final stop with `n`, `sum`, `report`, and `total` in scope.
    std::process::exit(0);
}
