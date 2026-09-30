// `sqlx::migrate!` embarque les migrations à la compilation : en ajouter une
// doit suffire à recompiler.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
