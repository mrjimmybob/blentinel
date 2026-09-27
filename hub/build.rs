// sqlx::migrate! embeds hub/migrations at compile time. Without this, adding a
// new migration file would not trigger a rebuild, and the hub would silently
// ship without it.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
