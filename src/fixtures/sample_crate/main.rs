// Fixture file for tree-sitter parsing tests.
// `Rect`, `area`, and `perimeter` are defined in shapes.rs.
// These symbols are intentionally not imported here because tree-sitter
// performs syntactic, not semantic, analysis and parses this file correctly
// even without the `use` declarations.
fn describe(r: &Rect) {
    let _a = area(r);
    let _p = perimeter(r);
}

fn run() {
    let r = Rect { width: 3.0, height: 4.0 };
    describe(&r);
}
