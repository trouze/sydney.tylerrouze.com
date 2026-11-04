// Templates are expanded at compile time by askama's derive macros (and
// migrations by sqlx::migrate!), but cargo only watches ../src for changes —
// so without this, editing a template would leave the binary serving the old
// markup until some Rust source touched. Force rebuilds on template/migration
// edits.
fn main() {
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=migrations");
}
